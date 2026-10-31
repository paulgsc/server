#!/usr/bin/env python3
"""Every variable the stack reads, from where each one is declared (#403).

Each binary's clap `Config` is its schema: its tests hold a copy of it in
`infra/env/<binary>.json` (`app_config::schema`, CFG7 #503). The compose files
declare the rest, as `${NAME}`, `${NAME:-default}` or `${NAME:?message}`, with
the comment directly above the line as its help. This script reads both and
never a second list:

  example [--check]       write example.env from them, or, with --check, fail
                          when the checked-in one differs (CI: a hand edit of
                          example.env fails, and so does a schema change that
                          did not regenerate it).
  list [--env-file F]     every variable, who reads it, its default, and
                          whether F (default: .env) or the environment sets it.
  preflight [--env-file F]
                          before `docker compose up` (CFG8 #504): names every
                          required variable that is missing, every value its
                          binary would refuse, and every "ours" variable F
                          sets (its value is the deployment file's, CFG9
                          #604), all at once, and exits 1 if there is any.
                          Warns about each line of F that nothing reads (a
                          renamed or retired variable).
  deploy                  holds every deployment file, deploy/<role>/<binary>.env,
                          to its binary's schema (CFG9 #604): it states every
                          "ours" variable and nothing else, each value one its
                          binary takes; and holds each labelled compose
                          service to reading its file and passing exactly its
                          secret and neutral variables from .env. CI runs it,
                          so a PR that adds an "ours" variable decides its
                          value in every deployment file.
  export FILE             FILE's lines as shell `export`s, skipping each
                          variable the environment already sets (`make dev`).

`preflight` asks `docker compose config` for what each service is given, so
it sees what compose will: `.env` interpolation, each `env_file` (the
service's deployment file), the service's own `environment:` over them, and
the shell's variables through it. A service whose container runs a binary of
ours says which with an `env-schema` label.

Stdlib only, so it runs on the host as it is. `--self-test` runs every part
against fixtures first, as scripts/check_artifact_retention.py does.
"""

from __future__ import annotations

import argparse
import ipaddress
import json
import os
import re
import subprocess
import sys
import textwrap
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_DIR = REPO_ROOT / "infra" / "env"
HEAD = SCHEMA_DIR / "example.env.head"
EXAMPLE = REPO_ROOT / "example.env"
DEPLOY_DIR = REPO_ROOT / "deploy"
# The deploy/ roles docker compose runs (DEPLOY in the box's .env). Every
# other role is run from source (`make dev`), where nothing but its file sets
# the wiring compose's own `environment:` gives a container.
COMPOSE_ROLES = ("home",)
COMPOSE = REPO_ROOT / "docker-compose.yml"
# The order binaries appear in example.env; any other schema follows, by name.
ORDER = ["file_host", "orchestrator", "some-obs"]
LABEL = "env-schema"

# Read by a dependency, not declared by any `Config` (README.md,
# "Configuration", ENV1's Tolerance): a line setting one is not stale.
DEPENDENCY_VARIABLES = re.compile(r"^(COMPOSE_\w+|DOCKER_\w+|TOKIO_WORKER_THREADS|POLARS_MAX_THREADS|RUST_BACKTRACE|OTEL_RESOURCE_ATTRIBUTES|OTEL_EXPORTER_OTLP_HEADERS|OTEL_BSP_\w+)$")

INTERPOLATION = re.compile(r"(?<!\$)\$\{([A-Za-z_][A-Za-z0-9_]*)(?:(:?)([-?+])((?:[^{}]|\{[^{}]*\})*))?\}")
INCLUDE = re.compile(r"^\s*-\s*path:\s*(\S+)\s*$")


@dataclass
class ComposeVariable:
    name: str
    required: bool = False
    default: str | None = None
    message: str = ""
    help: str = ""
    files: list[str] = field(default_factory=list)
    # `${NAME:?}` somewhere, which refuses an empty value too; `${NAME?}`
    # refuses only an unset one.
    nonempty: bool = False
    declared: bool = False
    named: bool = False


# ── Reading the declarations ───────────────────────────────────────────────


def load_schemas(directory: Path = SCHEMA_DIR) -> list[dict]:
    schemas = [json.loads(path.read_text()) for path in directory.glob("*.json")]
    rank = {name: index for index, name in enumerate(ORDER)}
    return sorted(schemas, key=lambda schema: (rank.get(schema["binary"], len(ORDER)), schema["binary"]))


def compose_files(compose: Path = COMPOSE) -> list[Path]:
    """docker-compose.yml and each file it includes, in that order."""
    files = [compose]
    for line in compose.read_text().splitlines():
        match = INCLUDE.match(line)
        if match:
            files.append((compose.parent / match.group(1)).resolve())
    return files


def compose_variables(files: list[tuple[str, str]]) -> dict[str, ComposeVariable]:
    """Every `${NAME...}` in `files`, as (display name, text) pairs.

    A variable's help is the comment block directly above a use: the first
    that names it, else the first with a default or a `?`, else the first, else
    the `?` message. One used with `?` anywhere is required everywhere:
    compose interpolates the whole project before it starts any service
    (#407), so it blocks them all.
    """
    found: dict[str, ComposeVariable] = {}
    for name, text in files:
        lines = text.splitlines()
        for number, line in enumerate(lines):
            if line.lstrip().startswith("#"):
                continue
            for match in INTERPOLATION.finditer(line):
                variable, colon, operator, argument = match.groups()
                entry = found.setdefault(variable, ComposeVariable(variable))
                if name not in entry.files:
                    entry.files.append(name)
                above = comment_above(lines, number)
                if not entry.named and re.search(rf"\b{variable}\b", above):
                    entry.named = entry.declared = True
                    entry.help = own_paragraph(above, variable)
                elif not entry.named and operator and not entry.declared:
                    entry.declared = True
                    entry.help = above
                elif not entry.named and not entry.declared and not entry.help:
                    entry.help = above
                if operator == "?":
                    entry.required = True
                    entry.nonempty = entry.nonempty or colon == ":"
                    entry.message = entry.message or argument
                elif operator == "-" and entry.default is None:
                    entry.default = argument
    return found


def own_paragraph(block: str, name: str) -> str:
    """The part of a comment block that is `name`'s: from its line that starts
    `NAME:` to the next that starts another `OTHER:`, else the whole block,
    so one block can declare two variables used on one line."""
    lines = block.splitlines()
    starts = [index for index, line in enumerate(lines) if line.startswith(f"{name}:")]
    if not starts:
        return block
    own = [lines[starts[0]]]
    for line in lines[starts[0] + 1 :]:
        if re.match(r"^[A-Z][A-Z0-9_]*: ", line):
            break
        own.append(line)
    return "\n".join(own).strip()


def comment_above(lines: list[str], number: int) -> str:
    block: list[str] = []
    for line in reversed(lines[:number]):
        stripped = line.strip()
        if not stripped.startswith("#"):
            break
        block.append(stripped.lstrip("#").strip())
    return "\n".join(reversed(block)).strip()


def read_compose(compose: Path = COMPOSE) -> dict[str, ComposeVariable]:
    files = compose_files(compose)
    return compose_variables([(str(path.relative_to(REPO_ROOT)), path.read_text()) for path in files])


# ── example.env ────────────────────────────────────────────────────────────

BANNER = """\
###############################################
#   GENERATED: do not edit this file by hand
###############################################
# `make env-schema` writes it from each binary's clap Config (checked in as
# infra/env/<binary>.json) and the compose files' ${...} variables, with the
# header in infra/env/example.env.head. CI fails when it differs. To change a
# variable's help or default, change it where it is declared and regenerate.
# `make env` lists every variable and which ones your .env sets; `make
# preflight` checks .env before `docker compose up`.
"""


def quote(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def comment(text: str) -> list[str]:
    """`text` as comment lines, each paragraph wrapped (clap joins a doc
    comment's lines into one)."""
    out: list[str] = []
    for line in text.splitlines():
        if not line.strip():
            out.append("#")
            continue
        out += ["# " + part for part in textwrap.wrap(line, 76, break_long_words=False, break_on_hyphens=False)]
    return out


def describe(variable: dict) -> list[str]:
    facts = []
    if variable["required"]:
        facts.append("Required.")
    if variable.get("required_when"):
        facts.append(f"Required while {variable['required_when']['variable']} is {variable['required_when']['value']}.")
    if variable.get("bounds"):
        bounds = variable["bounds"]
        facts.append(bound_text(bounds).capitalize() + "".join(f"{' while' if index == 0 else ' and'} {when['variable']} is {'set' if when['value'] == SET else when['value']}" for index, when in enumerate(bounds["when"])) + ".")
    if variable["kind"] == "enum":
        facts.append("One of: " + ", ".join(variable["values"]) + ".")
    elif variable["kind"] == "bool":
        facts.append("true or false.")
    if variable["delimiter"]:
        facts.append(f"A list, separated by {variable['delimiter']!r}.")
    if variable["secret"]:
        facts.append("Secret: keep it out of the repository.")
    return facts


def render_example(schemas: list[dict], compose: dict[str, ComposeVariable], head: str, deployed: set[str] | None = None) -> str:
    """example.env. `deployed`: the binaries some deploy/<role>/ holds a file
    for; another's "ours" variables are left out too, where it is deployed."""
    out = [BANNER.rstrip("\n"), "", head.rstrip("\n")]
    readers: dict[str, list[tuple[str, dict]]] = {}
    for schema in schemas:
        for variable in schema["variables"]:
            readers.setdefault(variable["name"], []).append((schema["binary"], variable))
    written: set[str] = set()
    for schema in schemas:
        out += ["", "###############################################", f"#   {schema['binary']}", "###############################################"]
        if schema["about"]:
            out.append(f"# {schema['about']}.")
        ours = {name for name, read in readers.items() if any(other.get("class") == "ours" for _, other in read)}
        decided = [variable["name"] for variable in schema["variables"] if variable["name"] in ours]
        if decided and schema["binary"] in (deployed if deployed is not None else {schema["binary"]}):
            out += comment(
                f"Decided per host in deploy/<role>/{schema['binary']}.env, never here (CFG9, #604), and the preflight refuses one set in .env: "
                + ", ".join(decided)
                + "."
            )
        elif decided:
            out += comment(f"Not set here, as \"ours\" (CFG9, #604): {', '.join(decided)}. No deploy/<role>/ holds {schema['binary']} yet, so they are set where it is deployed.")
        listed = [variable["name"] for variable in schema["variables"] if variable["name"] in written and variable["name"] not in ours]
        if listed:
            out += comment("Also reads " + ", ".join(listed) + ", listed above.")
        group = None
        for variable in schema["variables"]:
            name = variable["name"]
            if name in written or name in ours:
                continue
            written.add(name)
            if variable["group"] != group:
                group = variable["group"]
                if group:
                    out += ["", f"# ── {group} ──"]
            out.append("")
            out += comment(variable["help"])
            facts = describe(variable)
            others = [(binary, other) for binary, other in readers[name] if binary != schema["binary"]]
            if others:
                also = ", ".join(binary + ("" if other["default"] == variable["default"] else f" (default {other['default']})") for binary, other in others)
                facts.append(f"Also read by {also}.")
            if name in compose:
                used = compose[name]
                facts.append("docker compose also reads it: " + ", ".join(used.files) + (f" (default {used.default})" if used.default is not None else "") + ".")
            if facts:
                out += comment(" ".join(facts))
            default = variable["default"] or ""
            out.append(f"# {name}={quote(default)}")
    rest = [variable for name, variable in compose.items() if name not in written]
    if rest:
        out += [
            "",
            "###############################################",
            "#   docker compose",
            "###############################################",
            "# Read by docker compose itself, when it reads the compose files, not by",
            "# a binary. Shell variables win over these lines.",
        ]
        for variable in sorted(rest, key=lambda variable: variable.name):
            out.append("")
            if variable.help or variable.message:
                out += comment(variable.help or variable.message)
            facts = ["Required: every service waits on it." if variable.required else "", "Read in " + ", ".join(variable.files) + "."]
            out += comment(" ".join(fact for fact in facts if fact))
            value = variable.default or ""
            out.append(f"# {variable.name}={quote(value)}")
    return "\n".join(out) + "\n"


# ── .env files ─────────────────────────────────────────────────────────────

LINE = re.compile(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$")


def parse_env_entries(text: str) -> dict[str, tuple[str, bool]]:
    """Each variable a dotenv file sets, as compose and dotenvy read it, and
    whether it is literal (single-quoted, so never interpolated). A later line
    wins over an earlier one, as it does for both."""
    entries: dict[str, tuple[str, bool]] = {}
    for line in text.splitlines():
        if line.lstrip().startswith("#"):
            continue
        match = LINE.match(line)
        if not match:
            continue
        name, raw = match.groups()
        if raw[:1] == "'" and "'" in raw[1:]:
            entries[name] = (raw[1 : raw.index("'", 1)], True)
        elif raw[:1] == '"':
            closing = re.match(r'"((?:[^"\\]|\\.)*)"', raw)
            body = closing.group(1) if closing else raw[1:]
            entries[name] = (re.sub(r'\\(["\\$])', r"\1", body), False)
        else:
            entries[name] = (re.split(r"\s#", raw, maxsplit=1)[0].strip(), False)
    return entries


def parse_env(text: str) -> dict[str, str]:
    """The variables a dotenv file sets, as compose and dotenvy read them."""
    return {name: value for name, (value, _) in parse_env_entries(text).items()}


def referenced_inside(text: str) -> set[str]:
    """The names a dotenv file's own values use (`DATABASE_URL="${STORAGE}/x"`)."""
    return {match.group(1) for match in INTERPOLATION.finditer(text)} | set(re.findall(r"(?<!\$)\$([A-Za-z_][A-Za-z0-9_]*)", text))


# ── Checking a value as its binary will ────────────────────────────────────

RANGES = {"u16": 2**16, "u32": 2**32, "u64": 2**64, "usize": 2**64, "seconds": 2**64}


SET = "*"


def number(value: float) -> str:
    return str(int(value)) if float(value).is_integer() else str(value)


def bound_text(bounds: dict) -> str:
    """"at least 60", "at most 1", or "from 0 to 1", as the binary says it."""
    low, high = bounds.get("min"), bounds.get("max")
    if low is not None and high is not None:
        return f"from {number(low)} to {number(high)}"
    return f"at least {number(low)}" if low is not None else f"at most {number(high)}"


def bound_refusal(variable: dict, value: str, holds) -> str | None:
    """Why `variable`'s bounds refuse `value`, while `holds` says each of their
    conditions does, or None (`app_config::schema::unmet`, #580)."""
    bounds = variable.get("bounds")
    if not bounds or not all(holds(when) for when in bounds["when"]):
        return None
    try:
        current = float(value)
    except ValueError:
        return None
    low, high = bounds.get("min"), bounds.get("max")
    if (low is not None and current < low) or (high is not None and current > high):
        return "must be " + bound_text(bounds)
    return None


def refusal(variable: dict, value: str) -> str | None:
    """Why `variable`'s binary would refuse `value` at start, or None."""
    kind = variable["kind"]
    values = value.split(variable["delimiter"]) if variable["delimiter"] else [value]
    for item in values:
        if kind == "bool" and item not in ("true", "false"):
            return "must be true or false"
        if kind == "enum" and item not in variable["values"]:
            return "must be one of " + ", ".join(variable["values"])
        if kind in RANGES:
            if not re.fullmatch(r"\+?[0-9]+", item) or int(item) >= RANGES[kind]:
                return "must be a whole number from 0" + (" to 65535" if kind == "u16" else "")
        if kind == "number":
            try:
                float(item)
            except ValueError:
                return "must be a number"
        if kind == "socket_addr" and not is_socket_addr(item):
            return "must be an address and port, e.g. 0.0.0.0:9464"
    return None


def is_socket_addr(value: str) -> bool:
    match = re.fullmatch(r"\[([^\]]+)\]:([0-9]+)|([^:]+):([0-9]+)", value)
    if not match:
        return False
    host, port = (match.group(1), match.group(2)) if match.group(1) else (match.group(3), match.group(4))
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return False
    return int(port) < 2**16


# ── preflight ──────────────────────────────────────────────────────────────


@dataclass
class Gap:
    name: str
    reader: str
    help: str
    secret: bool = False
    problem: str = "missing"


@dataclass
class Report:
    missing: list[Gap] = field(default_factory=list)
    invalid: list[Gap] = field(default_factory=list)
    # "ours" variables the env file sets, where nothing may override the
    # deployment file.
    misplaced: list[Gap] = field(default_factory=list)
    unread: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


def compose_config() -> dict:
    """`docker compose config` as JSON, not interpolated.

    Not interpolated, so it reads even with a required variable unset (which
    stops the interpolated form outright, naming only that one): preflight
    interpolates each service's environment itself, from the env file and the
    shell, as compose would.
    """
    result = subprocess.run(["docker", "compose", "config", "--no-interpolate", "--format", "json"], cwd=REPO_ROOT, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "docker compose config failed")
    return json.loads(result.stdout)


def interpolate(value: str, given: dict[str, str]) -> str:
    """`value` as compose interpolates it, an unset `${NAME:?}` becoming empty."""

    def one(match: re.Match) -> str:
        name, colon, operator, argument = match.groups()
        current = given.get(name)
        unset = current is None or (colon == ":" and current == "")
        if operator == "-":
            return interpolate(argument, given) if unset else current
        if operator == "+":
            return "" if unset else interpolate(argument, given)
        return current or ""

    value = INTERPOLATION.sub(one, value)
    value = re.sub(r"(?<!\$)\$([A-Za-z_][A-Za-z0-9_]*)", lambda match: given.get(match.group(1), ""), value)
    return value.replace("$$", "$")


def labels(service: dict) -> dict[str, str]:
    found = service.get("labels") or {}
    if isinstance(found, list):
        found = dict(item.split("=", 1) for item in found if "=" in item)
    return found


def env_file_paths(service: dict) -> list[tuple[str, bool]]:
    """Each `env_file` of a service, its path as written and whether it is
    required, in the order compose reads them."""
    paths = []
    for entry in service.get("env_file") or []:
        if isinstance(entry, dict):
            paths.append((str(entry.get("path", "")), bool(entry.get("required", True))))
        else:
            paths.append((str(entry), True))
    return paths


def reads_env_file(service: dict) -> bool:
    return any(path.endswith("/.env") for path, _ in env_file_paths(service))


def resolve_path(raw: str, given: dict[str, str]) -> Path:
    """An `env_file` path as compose opens it. Uninterpolated, compose has
    already joined a path that starts with `${...}` to the compose file's
    directory, so that prefix goes first."""
    start = raw.find("${")
    return Path(interpolate(raw[start:] if start > 0 else raw, given))


def deploy_entries(service: dict, given: dict[str, str], read) -> tuple[dict[str, tuple[str, bool]], list[str]]:
    """The lines of every `env_file` of a service but `.env`, read with
    `read(path) -> text | None`, a later file's over an earlier one's, and the
    errors for a required one that is not there."""
    entries: dict[str, tuple[str, bool]] = {}
    errors = []
    for raw, required in env_file_paths(service):
        if raw.endswith("/.env"):
            continue
        path = resolve_path(raw, given)
        text = read(path)
        if text is None:
            if required:
                errors.append(f"{path} does not exist, and compose requires it (is DEPLOY right?)")
            continue
        entries.update(parse_env_entries(text))
    return entries, errors


def own_environment(service: dict) -> dict[str, str | None]:
    """A service's own `environment:`, a bare name (passed from the shell) as None."""
    own = service.get("environment") or {}
    if isinstance(own, list):
        own = dict(item.split("=", 1) if "=" in item else (item, None) for item in own)
    return own


def resolve_env(entries: dict[str, tuple[str, bool]], shell: dict[str, str]) -> dict[str, tuple[str, bool]]:
    """`entries` with each value interpolated as compose does, line by line:
    from the shell and the lines above it, so `B=${A}` then `C=${B}` gives C
    A's value. Every value comes back literal, interpolated once."""
    resolved: dict[str, tuple[str, bool]] = {}
    for name, (value, literal) in entries.items():
        known = {**{other: text for other, (text, _) in resolved.items()}, **shell}
        resolved[name] = (value if literal else interpolate(value, known), True)
    return resolved


def environment_of(service: dict, entries: dict[str, tuple[str, bool]], given: dict[str, str], deployed: dict[str, tuple[str, bool]] | None = None) -> dict[str, str]:
    """What the container is given: its deployment file's lines, the env
    file's when it reads that whole, and its own `environment:` over them."""
    environment = {name: value if literal else interpolate(value, given) for name, (value, literal) in (deployed or {}).items()}
    if reads_env_file(service):
        environment.update({name: value if literal else interpolate(value, given) for name, (value, literal) in entries.items()})
    for name, value in own_environment(service).items():
        if value is None:
            if name in given:
                environment[name] = given[name]
        else:
            environment[name] = interpolate(value, given)
    return environment


def read_text(path: Path) -> str | None:
    return path.read_text() if path.is_file() else None


def preflight(schemas: list[dict], compose: dict[str, ComposeVariable], env_text: str, shell: dict[str, str], raw: dict, read=read_text) -> Report:
    report = Report()
    entries = resolve_env(parse_env_entries(env_text), shell)
    env = {name: value for name, (value, _) in entries.items()}
    given = {**env, **shell}
    by_binary = {schema["binary"]: schema for schema in schemas}
    for variable in compose.values():
        if variable.required and (variable.name not in given or (variable.nonempty and given[variable.name] == "")):
            report.missing.append(Gap(variable.name, "docker compose (" + ", ".join(variable.files) + ")", variable.message or variable.help))
    for name, service in sorted(raw.get("services", {}).items()):
        binary = labels(service).get(LABEL)
        if not binary:
            continue
        schema = by_binary.get(binary)
        if schema is None:
            report.errors.append(f"service {name} names env-schema {binary}, and infra/env/ has no {binary}.json")
            continue
        deployed, missing_files = deploy_entries(service, given, read)
        report.errors += [f"service {name}: {error}" for error in missing_files]
        environment = environment_of(service, entries, given, deployed)
        for variable in schema["variables"]:
            if variable.get("class") == "ours" and variable["name"] in env and variable["name"] not in {gap.name for gap in report.misplaced}:
                where = f"deploy/<role>/{binary}.env"
                report.misplaced.append(Gap(variable["name"], f"{binary} (service {name})", variable["help"], problem=f"decided in {where}, which nothing may override; remove it from .env"))
        defaults = {variable["name"]: variable["default"] for variable in schema["variables"]}
        for variable in schema["variables"]:
            # Unset and empty differ: clap hands an empty value to its parser,
            # which takes it as text and refuses it as anything else.
            value = environment.get(variable["name"])
            reader = f"{binary} (service {name})"

            def holds(when: dict, environment=environment, defaults=defaults) -> bool:
                current = environment.get(when["variable"], defaults.get(when["variable"]))
                return bool((current or "").strip()) if when["value"] == SET else current == when["value"]

            condition = variable.get("required_when")
            if condition and environment.get(condition["variable"], defaults.get(condition["variable"])) == condition["value"] and not (value or "").strip():
                why = f"required while {condition['variable']} is {condition['value']}. "
                report.missing.append(Gap(variable["name"], reader, why + variable["help"], variable["secret"]))
                continue
            if value is None:
                if variable["required"]:
                    report.missing.append(Gap(variable["name"], reader, variable["help"], variable["secret"]))
                continue
            problem = refusal(variable, value) or bound_refusal(variable, value, holds)
            if problem:
                report.invalid.append(Gap(variable["name"], reader, variable["help"], variable["secret"], problem))
    read = {variable["name"] for schema in schemas for variable in schema["variables"]} | set(compose) | referenced_inside(env_text)
    report.unread = sorted(name for name in env if name not in read and not DEPENDENCY_VARIABLES.match(name))
    return report


def first_line(text: str) -> str:
    """The first sentence of `text`'s first paragraph."""
    line = text.strip().splitlines()[0] if text.strip() else ""
    sentence = re.split(r"(?<=[.;])\s", line, maxsplit=1)[0]
    return sentence if len(sentence) <= 100 else sentence[:97].rstrip() + "..."


def print_report(report: Report, env_file: Path) -> None:
    for error in report.errors:
        print(f"error: {error}")
    if report.missing:
        print(f"{len(report.missing)} required variable(s) missing from {env_file} and the environment:")
        for gap in report.missing:
            print(f"  {gap.name}  [{gap.reader}]  {first_line(gap.help)}")
    if report.invalid:
        print(f"{len(report.invalid)} value(s) their binary would refuse at start:")
        for gap in report.invalid:
            print(f"  {gap.name}  [{gap.reader}]  {gap.problem}")
    if report.misplaced:
        print(f"{len(report.misplaced)} \"ours\" variable(s) {env_file} sets, whose value the deployment file decides (CFG9):")
        for gap in report.misplaced:
            print(f"  {gap.name}  [{gap.reader}]  {gap.problem}")
    for name in report.unread:
        print(f"warning: {env_file} sets {name}, which nothing reads (renamed or retired?)")
    for warning in report.warnings:
        print(f"warning: {warning}")
    if not failed(report):
        print(f"preflight: every required variable is set, every value parses, and {env_file} overrides no deployment file.")


def failed(report: Report) -> bool:
    return bool(report.missing or report.invalid or report.misplaced or report.errors)


# ── deploy/<role>/<binary>.env ─────────────────────────────────────────────

UNSET = re.compile(r"^#unset\s+([A-Za-z_][A-Za-z0-9_]*)\s*$")


def deployment_lines(text: str) -> tuple[dict[str, str], set[str], list[str]]:
    """A deployment file's values, its `#unset NAME` lines, and why any other
    line is not one of those, a comment or blank."""
    values: dict[str, str] = {}
    unset: set[str] = set()
    problems = []
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        marker = UNSET.match(stripped)
        name = marker.group(1) if marker else None
        if not marker:
            if not stripped or stripped.startswith("#"):
                continue
            match = LINE.match(line)
            if not match:
                problems.append(f"line {number} is not NAME=value, `#unset NAME`, a comment or blank")
                continue
            name = match.group(1)
            if "$" in match.group(2):
                problems.append(f"{name} holds a `$`: compose would interpolate it from the box's .env, so the value would not be the one reviewed here")
        if name in values or name in unset:
            problems.append(f"{name} is stated twice")
        if marker:
            unset.add(name)
        else:
            values[name] = parse_env(line)[name]
    return values, unset, problems


def compose_sets(service: dict) -> dict[str, str]:
    """What a service's own `environment:` sets to a literal value, with no
    `${...}`: compose's own network, which wins over the deployment file."""
    return {name: value for name, value in own_environment(service).items() if value is not None and "$" not in value}


def check_deployment(role: str, binary: str, schema: dict, text: str, service: dict | None) -> list[str]:
    """Why `deploy/<role>/<binary>.env` is not every "ours" value of `binary`,
    decided: each stated once (a value, or `#unset` where unset is a choice),
    nothing else, and each one its binary takes at start."""
    where = f"deploy/{role}/{binary}.env"
    values, unset, problems = deployment_lines(text)
    variables = {variable["name"]: variable for variable in schema["variables"]}
    fixed = compose_sets(service) if service is not None else {}
    for name in dict.fromkeys([*values, *sorted(unset)]):
        variable = variables.get(name)
        if variable is None:
            problems.append(f"{name} is not a variable of {binary} (infra/env/{binary}.json)")
        elif variable["class"] == "secret":
            problems.append(f"{name} is a secret: its value stays off git, in the box's .env")
        elif variable["class"] != "ours":
            problems.append(f"{name} is neutral: an override goes in the box's .env, not here")
        elif name in fixed:
            problems.append(f"{name} is set by compose's own `environment:` ({fixed[name]}), so a line here never reaches the container")
        elif name in unset and (variable["required"] or variable["default"] is not None):
            problems.append(f"{name} cannot be left unset: " + ("it is required" if variable["required"] else f"it defaults to {variable['default']}; state the value"))
    effective = {**{name: variable["default"] for name, variable in variables.items() if variable["default"] is not None}, **values, **fixed}

    def holds(when: dict) -> bool:
        current = effective.get(when["variable"])
        return bool((current or "").strip()) if when["value"] == SET else current == when["value"]

    for variable in schema["variables"]:
        name = variable["name"]
        if variable["class"] != "ours" or name in fixed:
            continue
        if name not in values and name not in unset:
            problems.append(f"{name} is not stated: decide its value here" + ("" if variable["required"] or variable["default"] is not None else f", or `#unset {name}`"))
            continue
        value = values.get(name)
        condition = variable.get("required_when")
        if condition and holds({"variable": condition["variable"], "value": condition["value"]}) and not (value or "").strip():
            problems.append(f"{name} is required while {condition['variable']} is {condition['value']}")
        if value is None:
            continue
        if variable["required"] and not value.strip():
            problems.append(f"{name} is required, and empty")
            continue
        problem = refusal(variable, value) or bound_refusal(variable, value, holds)
        if problem:
            problems.append(f"{name}={value} {problem}")
    return [f"{where}: {problem}" for problem in problems]


def check_service(name: str, binary: str, schema: dict, service: dict) -> list[str]:
    """Why a labelled compose service does not take `binary`'s "ours" values
    from its deployment file and only its secrets and neutral overrides from
    the box's .env."""
    problems = []
    if reads_env_file(service):
        problems.append("reads the whole .env (`env_file`), so every line of it reaches the container")
    if not any(required and "/deploy/${DEPLOY" in path and path.endswith(f"}}/{binary}.env") for path, required in env_file_paths(service)):
        problems.append(f"does not read deploy/${{DEPLOY:?...}}/{binary}.env (`env_file`, required)")
    own = own_environment(service)
    for variable in schema["variables"]:
        variable_name = variable["name"]
        if variable["class"] == "ours":
            value = own.get(variable_name, "")
            if variable_name in own and (value is None or "$" in value):
                problems.append(f"passes {variable_name} from .env (`environment:`), and it is ours: its value is the deployment file's")
        elif variable_name not in own:
            problems.append(f"does not pass {variable_name} ({variable['class']}) from .env: add `- {variable_name}` to its `environment:`")
    return [f"service {name}: {problem}" for problem in problems]


def check_deploy(schemas: list[dict], raw: dict, files: dict[tuple[str, str], str]) -> list[str]:
    """Every deployment file, `files[(role, binary)]`, held to its binary's
    schema, and every labelled service of `raw` to reading one: the CI check
    that a PR adding an "ours" variable decides it in every deployment file."""
    by_binary = {schema["binary"]: schema for schema in schemas}
    services = {}
    problems = []
    for name, service in sorted(raw.get("services", {}).items()):
        binary = labels(service).get(LABEL)
        if not binary:
            continue
        if binary not in by_binary:
            problems.append(f"service {name} names env-schema {binary}, and infra/env/ has no {binary}.json")
            continue
        services[binary] = service
        problems += check_service(name, binary, by_binary[binary], service)
    roles = sorted({role for role, _ in files} | set(COMPOSE_ROLES))
    for role in roles:
        if role in COMPOSE_ROLES:
            problems += [f"deploy/{role}/{binary}.env is missing: compose runs {binary} there" for binary in sorted(services) if (role, binary) not in files]
        for (file_role, binary), text in sorted(files.items()):
            if file_role != role:
                continue
            if binary not in by_binary:
                problems.append(f"deploy/{role}/{binary}.env names no binary in infra/env/")
                continue
            problems += check_deployment(role, binary, by_binary[binary], text, services.get(binary) if role in COMPOSE_ROLES else None)
    return problems


def read_deployments(directory: Path = DEPLOY_DIR) -> dict[tuple[str, str], str]:
    return {(path.parent.name, path.stem): path.read_text() for path in sorted(directory.glob("*/*.env"))}


def shell_quote(value: str) -> str:
    return "'" + value.replace("'", "'\\''") + "'"


def render_exports(text: str, environment: dict[str, str]) -> str:
    """A deployment file as `export NAME='value'` lines for a shell to `eval`,
    skipping every variable `environment` already sets."""
    values, _, _ = deployment_lines(text)
    return "".join(f"export {name}={shell_quote(value)}\n" for name, value in values.items() if name not in environment)


# ── list ───────────────────────────────────────────────────────────────────


def render_list(schemas: list[dict], compose: dict[str, ComposeVariable], given: dict[str, str]) -> str:
    rows = []
    seen = set()
    for schema in schemas:
        for variable in schema["variables"]:
            readers = [other["binary"] for other in schemas if any(v["name"] == variable["name"] for v in other["variables"])]
            if variable["name"] in seen:
                continue
            seen.add(variable["name"])
            condition = variable.get("required_when")
            when = f"  required while {condition['variable']} is {condition['value']}" if condition else ""
            rows.append((variable["name"], ", ".join(readers), "required" if variable["required"] else (variable["default"] if variable["default"] is not None else ""), when, variable["help"]))
    for variable in sorted(compose.values(), key=lambda variable: variable.name):
        if variable.name not in seen:
            rows.append((variable.name, "docker compose", "required" if variable.required else (variable.default or ""), "", variable.help))
    out = []
    for name, readers, default, when, help_text in rows:
        state = "set  " if name in given else ("UNSET" if default == "required" else "     ")
        out.append(f"{state} {name}  [{readers}]" + (f"  default: {default}" if default and default != "required" else ("  required" if default == "required" else "")) + when)
        if first_line(help_text):
            out.append(f"        {first_line(help_text)}")
    return "\n".join(out) + "\n"


# ── self-test ──────────────────────────────────────────────────────────────


def self_test() -> None:
    def var(name: str, kind: str = "string", **extra) -> dict:
        variable = {"name": name, "group": None, "required": False, "default": None, "kind": kind, "values": [], "delimiter": None, "secret": False, "help": f"{name}'s help.", **extra}
        return {"class": "secret" if variable["secret"] else "neutral", **variable}

    schema = {
        "binary": "probe",
        "about": "A probe",
        "variables": [
            var("DB", required=True, secret=True),
            var("TOKEN", required=True),
            var("KEY", required=True),
            var("ON", "bool", group="Switches", default="false"),
            var("PORT", "u16", group="Switches", default="3000"),
            var("MODE", "enum", group="Switches", default="a", values=["a", "b"]),
            var("ADDR", "socket_addr", group="Switches", default="0.0.0.0:1"),
            var("LOG", group="Switches", default="info"),
            var("RP_ID", required_when={"variable": "AUTH", "value": "true"}),
            var("AUTH", "bool", default="true"),
            var("PWD", required=True, secret=True),
        ],
    }
    other = {"binary": "other", "about": "", "variables": [{**schema["variables"][7], "default": "other=debug"}]}
    compose = compose_variables(
        [
            ("a.yml", "# Not about it.\n      - ${ROOT}/x\n      - path: ${ROOT:?set ROOT}/.env\n      - LOG=${LOG:-info}\n      # x\n      - $${NOT_A_VARIABLE}\n      - ${MODELS:-/m}:/models\n"),
            ("b.yml", "      # ROOT: the checkout's root.\n      - ${ROOT:?set ROOT}/data\n      - ${BARE?set BARE}\n# - ${COMMENTED_OUT:?no}\n"),
        ]
    )
    assert set(compose) == {"ROOT", "LOG", "MODELS", "BARE"}, compose
    assert compose["BARE"].help == "" and compose["BARE"].message == "set BARE" and not compose["BARE"].nonempty
    assert compose["ROOT"].required and compose["ROOT"].nonempty and compose["ROOT"].files == ["a.yml", "b.yml"]
    assert compose["ROOT"].help == "ROOT: the checkout's root.", "the comment that names a variable is its help"
    assert compose["MODELS"].default == "/m" and not compose["MODELS"].required
    pair = compose_variables([("c.yml", "      # Why.\n      # A: the first.\n      # More of A.\n      # B: the second.\n      - ${A:?a}/${B:?b}\n")])
    assert pair["A"].help == "A: the first.\nMore of A." and pair["B"].help == "B: the second.", (pair["A"].help, pair["B"].help)

    example = render_example([schema, other], compose, "# head")
    flat = " ".join(line.lstrip("#").strip() for line in example.splitlines())
    assert '# DB=""' in example and '# ON="false"' in example and "# ── Switches ──" in example, example
    assert "Also read by other (default other=debug)." in flat and "docker compose also reads it: a.yml (default info)." in flat, example
    assert "# Also reads LOG, listed above." in example and "Required while AUTH is true." in flat, example
    assert example.count("LOG=") == 1, "a variable shared by two binaries is written once"
    assert '# ROOT=""' in example and '# MODELS="/m"' in example and "Secret: keep it out" in example
    assert parse_env(example) == {}, "every line is commented out, so a copy of it sets nothing"

    env_text = "\n".join(
        [
            'DB="sqlite://x" # the dev one',
            "PORT=70000",
            "PORT=70001",
            "MODE=c",
            "ADDR=localhost:1",
            "ON=",
            "PWD=''",
            "WHISPER_MODEL=old",
            "STORAGE=/s",
            'DBX="${STORAGE}/y"',
            "LIT='$STORAGE'",
            "TOKIO_WORKER_THREADS=4",
            "COMPOSE_PROJECT_NAME=x",
            "# KEY=commented",
            "",
        ]
    )
    assert parse_env('A="x # y"\nB=z # c\nexport C=\'q\'\nD="3000" # d\nE="a\\"b"\n') == {"A": "x # y", "B": "z", "C": "q", "D": "3000", "E": 'a"b'}
    raw = {
        "services": {
            "app": {
                "labels": {LABEL: "probe"},
                "env_file": [{"path": "/repo/infra/compose/${ROOT:?x}/.env"}],
                "environment": ["TOKEN=${TOKEN_FROM_SHELL}", "LOG=${LOG:-info}", "KEY", "MODE=b"],
            },
            "db": {"environment": {}},
        }
    }
    report = preflight([schema, other], compose, env_text, {}, raw)
    assert [gap.name for gap in report.missing] == ["ROOT", "BARE", "KEY", "RP_ID"], "every gap at once, a condition's default included"
    assert [gap.name for gap in report.invalid] == ["ON", "PORT", "ADDR"], "an empty value is refused as a bool, and the last of two lines counts"
    assert report.unread == ["DBX", "LIT", "WHISPER_MODEL"], report.unread
    assert not any(gap.name in ("TOKEN", "PWD") for gap in report.missing), "set but empty is set: clap takes it as text"
    shelled = preflight([schema], compose, env_text + "AUTH=false\n", {"ROOT": "/r", "BARE": "", "KEY": "k"}, raw)
    assert not shelled.missing, "the shell's variables count, `${X?}` allows an empty one, and a condition that is off asks for nothing"
    assert not preflight([schema], compose, env_text, {"ROOT": "/r", "BARE": "b", "KEY": "k"}, {"services": {"app": {"environment": {}}}}).missing, "a service with no env-schema label is not checked"
    chained = resolve_env(parse_env_entries("A=false\nB=${A}\nC=${B}\nD='${A}'\n"), {})
    assert {name: value for name, (value, _) in chained.items()} == {"A": "false", "B": "false", "C": "false", "D": "${A}"}, chained
    assert resolve_env(parse_env_entries("A=x\nB=${A}\n"), {"A": "shell"})["B"] == ("shell", True), "the shell wins over a line"
    literal = environment_of(raw["services"]["app"], parse_env_entries(env_text), {"STORAGE": "/s"})
    assert literal["LIT"] == "$STORAGE" and literal["DBX"] == "/s/y" and literal["MODE"] == "b", literal
    assert refusal(schema["variables"][3], "on") and refusal(schema["variables"][3], "true") is None
    assert refusal(schema["variables"][6], "[::1]:80") is None and refusal(schema["variables"][6], "0.0.0.0") is not None
    assert interpolate("${A:-x}/${B-y}/${C:+z}/$D/$$E/${F:?m}", {"A": "", "B": "", "C": "1", "D": "d"}) == "x//z/d/$E/"

    import tempfile

    gated = var("SECONDS", "u64", default="60", bounds={"min": 60, "max": None, "when": [{"variable": "ON", "value": "true"}]})
    days = var("DAYS", "u64", default="90", bounds={"min": 30, "max": 100, "when": [{"variable": "ON", "value": "true"}, {"variable": "TOKEN", "value": SET}]})
    bounded = {"binary": "probe", "about": "", "variables": [var("ON", "bool", default="false"), var("TOKEN"), gated, days]}
    service = {"services": {"app": {"labels": {LABEL: "probe"}, "env_file": [{"path": "/x/.env"}]}}}
    off = preflight([bounded], {}, "SECONDS=1\nDAYS=1\n", {}, service)
    assert not off.invalid, "a bound whose condition is off holds nothing"
    on = preflight([bounded], {}, "ON=true\nSECONDS=59\nDAYS=101\n", {}, service)
    assert [(gap.name, gap.problem) for gap in on.invalid] == [("SECONDS", "must be at least 60")], "every condition must hold, an unset TOKEN included"
    both = preflight([bounded], {}, "ON=true\nTOKEN=t\nDAYS=101\n", {}, service)
    assert [(gap.name, gap.problem) for gap in both.invalid] == [("DAYS", "must be from 30 to 100")], both.invalid
    assert "At least 60 while ON is true." in " ".join(describe(gated)) and "From 30 to 100 while ON is true and TOKEN is set." in " ".join(describe(days))
    # CFG9 (#604): the deployment file reaches the service, an "ours" value
    # in .env is refused, and example.env leaves "ours" variables out.
    ours = {
        "binary": "probe",
        "about": "",
        "variables": [
            var("ON", "bool", default="false", **{"class": "ours"}),
            var("URL", required_when={"variable": "ON", "value": "true"}, **{"class": "ours"}),
            var("WHO", **{"class": "ours"}),
            var("DAYS", "u64", default="90", bounds={"min": 30, "max": None, "when": [{"variable": "ON", "value": "true"}]}, **{"class": "ours"}),
            var("WIRE", required=True, **{"class": "ours"}),
            var("PORT", "u16", default="3000"),
            var("KEY", secret=True),
        ],
    }
    good = "# a comment\nON=true\nURL=https://x\n#unset WHO\nDAYS=30\n"
    service = {
        "labels": {LABEL: "probe"},
        "env_file": [{"path": "/repo/infra/compose/${ROOT:?x}/deploy/${DEPLOY:?set DEPLOY}/probe.env", "required": True}],
        "environment": ["WIRE=w://wire", "PORT", "KEY"],
    }
    raw = {"services": {"app": service}}
    files = {"/r/deploy/home/probe.env": good}
    deployed = preflight([ours], {}, "PORT=4000\nKEY=k\n", {"ROOT": "/r", "DEPLOY": "home"}, raw, read=lambda path: files.get(str(path)))
    assert not (deployed.missing or deployed.invalid or deployed.misplaced or deployed.errors), deployed
    assert environment_of(service, {}, {"ROOT": "/r", "DEPLOY": "home", "PORT": "4000"}, parse_env_entries(good))["ON"] == "true"
    misplaced = preflight([ours], {}, "ON=false\nPORT=1\n", {"ROOT": "/r", "DEPLOY": "home"}, raw, read=lambda path: files.get(str(path)))
    assert [gap.name for gap in misplaced.misplaced] == ["ON"] and failed(misplaced), misplaced
    lost = preflight([ours], {}, "", {"ROOT": "/r", "DEPLOY": "nowhere"}, raw, read=lambda path: files.get(str(path)))
    assert lost.errors and "/r/deploy/nowhere/probe.env does not exist" in lost.errors[0], lost.errors
    assert resolve_path("/repo/infra/compose/${ROOT:?x}/deploy/${DEPLOY:?y}/probe.env", {"ROOT": "/r", "DEPLOY": "dev"}) == Path("/r/deploy/dev/probe.env")
    assert "No deploy/<role>/ holds probe yet" in " ".join(line.lstrip("# ") for line in render_example([ours], {}, "# head", set()).splitlines())
    example = render_example([ours], {}, "# head")
    assert "ON=" not in example and "# PORT=" in example and "never here" in example and "ON, URL, WHO, DAYS, WIRE." in " ".join(line.lstrip("# ") for line in example.splitlines()), example

    assert check_deploy([ours], raw, {("home", "probe"): good, ("dev", "probe"): good + "WIRE=sqlite://dev.db\n"}) == []
    problems = check_deploy(
        [ours],
        raw,
        {("home", "probe"): "ON=true\nDAYS=29\nWIRE=x\nPORT=1\nKEY=s\nGONE=1\n#unset WHO\n#unset WHO\nX=${Y}\nnot a line\n", ("dev", "probe"): "ON=yes\n#unset DAYS\n", ("dev", "nobody"): ""},
    )
    expected = [
        "deploy/home/probe.env: line 10 is not NAME=value, `#unset NAME`, a comment or blank",
        "deploy/home/probe.env: WHO is stated twice",
        "deploy/home/probe.env: X holds a `$`",
        "deploy/home/probe.env: WIRE is set by compose's own `environment:` (w://wire)",
        "deploy/home/probe.env: PORT is neutral",
        "deploy/home/probe.env: KEY is a secret",
        "deploy/home/probe.env: GONE is not a variable of probe",
        "deploy/home/probe.env: X is not a variable of probe",
        "deploy/home/probe.env: URL is not stated: decide its value here, or `#unset URL`",
        "deploy/home/probe.env: DAYS=29 must be at least 30",
        "deploy/dev/probe.env: DAYS cannot be left unset: it defaults to 90",
        "deploy/dev/probe.env: URL is not stated",
        "deploy/dev/probe.env: WHO is not stated",
        "deploy/dev/probe.env: ON=yes must be true or false",
        "deploy/dev/probe.env: WIRE is not stated: decide its value here",
        "deploy/dev/nobody.env names no binary",
    ]
    for want in expected:
        assert sum(problem.startswith(want) for problem in problems) == 1, (want, problems)
    assert len(problems) == len(expected), problems
    unmet = check_deploy([ours], raw, {("home", "probe"): "ON=true\n#unset URL\n#unset WHO\nDAYS=30\n"})
    assert unmet == ["deploy/home/probe.env: URL is required while ON is true"], unmet
    assert check_deploy([ours], raw, {}) == ["deploy/home/probe.env is missing: compose runs probe there"]
    leaky = {"labels": {LABEL: "probe"}, "env_file": [{"path": "/x/${ROOT:?x}/.env"}], "environment": {"ON": None, "URL": "${URL:-u}", "PORT": None}}
    assert check_service("app", "probe", ours, leaky) == [
        "service app: reads the whole .env (`env_file`), so every line of it reaches the container",
        "service app: does not read deploy/${DEPLOY:?...}/probe.env (`env_file`, required)",
        "service app: passes ON from .env (`environment:`), and it is ours: its value is the deployment file's",
        "service app: passes URL from .env (`environment:`), and it is ours: its value is the deployment file's",
        "service app: does not pass KEY (secret) from .env: add `- KEY` to its `environment:`",
    ], check_service("app", "probe", ours, leaky)
    assert render_exports("A=1\nB=\"x y; z\"\n#unset C\nD=it's\n", {"A": "shell"}) == "export B='x y; z'\nexport D='it'\\''s'\n"
    assert other_env_file(REPO_ROOT / ".env", None) is None and other_env_file(Path("/elsewhere/.env"), None)
    assert other_env_file(Path("/srv/box/.env"), "/srv/box") is None, "$REPO_ROOT/.env is the one compose reads"

    listing = render_list([schema, other], compose, {"DB": "x"})
    assert "set   DB  [probe]" in listing and "UNSET TOKEN" in listing and "LOG  [probe, other]" in listing, listing
    assert "RP_ID  [probe]  required while AUTH is true" in listing, listing


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--self-test", action="store_true", help="test this script against fixtures first")
    commands = parser.add_subparsers(dest="command")
    example = commands.add_parser("example", help="write example.env")
    example.add_argument("--check", action="store_true", help="fail when example.env is not what it would write")
    listing = commands.add_parser("list", help="list every variable the stack reads")
    listing.add_argument("--env-file", type=Path, default=REPO_ROOT / ".env")
    check = commands.add_parser("preflight", help="check .env before docker compose up")
    check.add_argument("--env-file", type=Path, default=REPO_ROOT / ".env", help="the .env to check (default: the repo's, which is the one compose reads)")
    commands.add_parser("deploy", help="hold every deploy/<role>/<binary>.env to its schema, and compose to reading it")
    exports = commands.add_parser("export", help="print a deployment file as shell exports, skipping what the environment sets")
    exports.add_argument("file", type=Path)
    args = parser.parse_args()

    if args.self_test:
        self_test()
        print("env_schema self-test: ok")
        if args.command is None:
            return 0
    schemas = load_schemas()
    compose = read_compose()

    if args.command == "example":
        rendered = render_example(schemas, compose, HEAD.read_text(), {binary for _, binary in read_deployments()})
        if args.check:
            if EXAMPLE.read_text() != rendered:
                print("example.env is not what `make env-schema` writes from infra/env/*.json and the compose files.")
                print("Change the variable where it is declared (its Config, or the compose file), then run `make env-schema`.")
                return 1
            print("example.env matches its declarations.")
            return 0
        EXAMPLE.write_text(rendered)
        return 0
    if args.command == "deploy":
        try:
            raw = compose_config()
        except (RuntimeError, OSError) as error:
            print(f"docker compose config failed, so no service was checked: {error}")
            return 1
        problems = check_deploy(schemas, raw, read_deployments())
        for problem in problems:
            print(problem)
        if problems:
            print(f"{len(problems)} problem(s): every \"ours\" variable is decided in every deploy/<role>/<binary>.env (CFG9, #604).")
            return 1
        print(f"every deployment file states every \"ours\" value of its binary ({', '.join(f'{role}/{binary}' for role, binary in read_deployments())}).")
        return 0
    if args.command == "export":
        sys.stdout.write(render_exports(args.file.read_text(), dict(os.environ)))
        return 0

    env_file: Path = args.env_file
    env_text = env_file.read_text() if env_file.exists() else ""
    if args.command == "list":
        sys.stdout.write(render_list(schemas, compose, {**parse_env(env_text), **os.environ}))
        return 0
    if args.command == "preflight":
        report = run_preflight(schemas, compose, env_file)
        print_report(report, env_file)
        return 1 if failed(report) else 0
    parser.print_help()
    return 2


def other_env_file(env_file: Path, repo_root: str | None) -> str | None:
    """A warning when `env_file` is not the one compose reads: the repo's
    `.env`, and with it `$REPO_ROOT/.env` (#580)."""
    read = {(REPO_ROOT / ".env").resolve()}
    if repo_root:
        read.add((Path(repo_root) / ".env").resolve())
    if env_file.resolve() in read:
        return None
    return f"{env_file} is not the .env docker compose reads ({REPO_ROOT / '.env'}): this checks {env_file}, and compose starts on the other"


def run_preflight(schemas: list[dict], compose: dict[str, ComposeVariable], env_file: Path) -> Report:
    env_text = env_file.read_text() if env_file.exists() else ""
    shell = dict(os.environ)
    elsewhere = other_env_file(env_file, {**parse_env(env_text), **shell}.get("REPO_ROOT"))
    try:
        raw = compose_config()
    except (RuntimeError, OSError) as error:
        report = preflight(schemas, compose, env_text, shell, {})
        report.errors.append(f"docker compose config failed, so no service was checked: {error}")
        report.warnings += [elsewhere] if elsewhere else []
        return report
    report = preflight(schemas, compose, env_text, shell, raw)
    report.warnings += [elsewhere] if elsewhere else []
    return report


if __name__ == "__main__":
    sys.exit(main())
