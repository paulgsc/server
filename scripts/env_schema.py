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
  preflight [--env-file F] [--fill]
                          before `docker compose up` (CFG8 #504): names every
                          required variable that is missing and every value
                          its binary would refuse, all at once, and exits 1 if
                          there is any. Warns about each line of F that nothing
                          reads (a renamed or retired variable). With --fill,
                          asks for each missing value, hidden for a secret, and
                          writes it to F, keeping every other line as it was.

`preflight` asks `docker compose config` for what each service is given, so
it sees what compose will: `.env` interpolation, `env_file`, the service's
own `environment:`, and the shell's variables over all of them. A service
whose container runs a binary of ours says which with an `env-schema` label.

Stdlib only, so it runs on the host as it is. `--self-test` runs every part
against fixtures first, as scripts/check_artifact_retention.py does.
"""

from __future__ import annotations

import argparse
import getpass
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
                    entry.help = above
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


def render_example(schemas: list[dict], compose: dict[str, ComposeVariable], head: str) -> str:
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
        listed = [variable["name"] for variable in schema["variables"] if variable["name"] in written]
        if listed:
            out += comment("Also reads " + ", ".join(listed) + ", listed above.")
        group = None
        for variable in schema["variables"]:
            name = variable["name"]
            if name in written:
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


def env_line(name: str, value: str) -> str | None:
    """`name=value` as a dotenv line both compose and dotenvy read back as
    `value`: single-quoted, so a `$` in it is never interpolated. None for a
    value holding a single quote, which cannot be written that way."""
    return None if "'" in value else f"{name}='{value}'"


def referenced_inside(text: str) -> set[str]:
    """The names a dotenv file's own values use (`DATABASE_URL="${STORAGE}/x"`)."""
    return {match.group(1) for match in INTERPOLATION.finditer(text)} | set(re.findall(r"(?<!\$)\$([A-Za-z_][A-Za-z0-9_]*)", text))


def set_line(text: str, name: str, line: str) -> str:
    """`text` with `name` set by `line`: its last line (the one that counts)
    replaced in place, or `line` added."""
    lines = text.splitlines()
    for index in reversed(range(len(lines))):
        match = LINE.match(lines[index])
        if match and match.group(1) == name and not lines[index].lstrip().startswith("#"):
            lines[index] = line
            return "\n".join(lines) + "\n"
    return "\n".join([*lines, line]) + "\n"


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
    # Whether writing it to the env file reaches its reader.
    fillable: bool = True


@dataclass
class Report:
    missing: list[Gap] = field(default_factory=list)
    invalid: list[Gap] = field(default_factory=list)
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


def reads_env_file(service: dict) -> bool:
    files = service.get("env_file") or []
    return any(str(entry.get("path", entry) if isinstance(entry, dict) else entry).endswith("/.env") for entry in files)


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


def environment_of(service: dict, entries: dict[str, tuple[str, bool]], given: dict[str, str]) -> dict[str, str]:
    """What the container is given: the env file's lines when it reads it, and
    its own `environment:` over them."""
    environment = {name: value if literal else interpolate(value, given) for name, (value, literal) in entries.items()} if reads_env_file(service) else {}
    for name, value in own_environment(service).items():
        if value is None:
            if name in given:
                environment[name] = given[name]
        else:
            environment[name] = interpolate(value, given)
    return environment


def reaches(service: dict, name: str) -> bool:
    """Whether a line setting `name` in `.env` reaches the service's container:
    through `env_file`, unless its own `environment:` sets `name` without
    reading it, or through that line's `${name}`."""
    own = own_environment(service)
    if name in own:
        value = own[name]
        return value is None or re.search(rf"\$\{{?{name}\b", value) is not None
    return reads_env_file(service)


def preflight(schemas: list[dict], compose: dict[str, ComposeVariable], env_text: str, shell: dict[str, str], raw: dict) -> Report:
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
        environment = environment_of(service, entries, given)
        defaults = {variable["name"]: variable["default"] for variable in schema["variables"]}
        for variable in schema["variables"]:
            # Unset and empty differ: clap hands an empty value to its parser,
            # which takes it as text and refuses it as anything else.
            value = environment.get(variable["name"])
            reader = f"{binary} (service {name})"
            fillable = reaches(service, variable["name"])
            condition = variable.get("required_when")
            if condition and environment.get(condition["variable"], defaults.get(condition["variable"])) == condition["value"] and not (value or "").strip():
                why = f"required while {condition['variable']} is {condition['value']}. "
                report.missing.append(Gap(variable["name"], reader, why + variable["help"], variable["secret"], fillable=fillable))
                continue
            if value is None:
                if variable["required"]:
                    report.missing.append(Gap(variable["name"], reader, variable["help"], variable["secret"], fillable=fillable))
                continue
            def holds(when: dict) -> bool:
                current = environment.get(when["variable"], defaults.get(when["variable"]))
                return bool((current or "").strip()) if when["value"] == SET else current == when["value"]

            problem = refusal(variable, value) or bound_refusal(variable, value, holds)
            if problem:
                report.invalid.append(Gap(variable["name"], reader, variable["help"], variable["secret"], problem, fillable))
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
    for name in report.unread:
        print(f"warning: {env_file} sets {name}, which nothing reads (renamed or retired?)")
    for warning in report.warnings:
        print(f"warning: {warning}")
    if not (report.missing or report.invalid or report.errors):
        print(f"preflight: every required variable is set, and every value parses ({env_file}).")


def fill(report: Report, env_file: Path, schemas: list[dict], ask=input, ask_secret=getpass.getpass, say=print) -> list[str]:
    """Asks for each missing or refused value and writes it to `env_file`."""
    variables = {variable["name"]: variable for schema in schemas for variable in schema["variables"]}
    text = env_file.read_text() if env_file.exists() else ""
    skipped = []
    for gap in [*report.missing, *report.invalid]:
        if not gap.fillable:
            skipped.append(f"{gap.name}: a line in .env does not reach {gap.reader}; set it in that service's compose file")
            continue
        if first_line(gap.help):
            say(f"\n{gap.name}: {first_line(gap.help)}")
        while True:
            value = (ask_secret if gap.secret else ask)(f"{gap.name}=")
            known = variables.get(gap.name)
            problem = (refusal(known, value) or bound_refusal(known, value, lambda _: True)) if known and value else None
            if value and not problem and env_line(gap.name, value):
                break
            say(f"  {problem or ('a value is required' if not value else 'a value with a single quote: write it to the file yourself')}")
        text = set_line(text, gap.name, env_line(gap.name, value) or "")
    # In clear text: .env is where compose reads secrets from until #408's
    # agenix or sops-nix. So it is made readable by its owner alone, which
    # compose, running as that owner, still is.
    descriptor = os.open(env_file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(descriptor, 0o600)
    with os.fdopen(descriptor, "w") as out:
        out.write(text)
    return skipped


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
        return {"name": name, "group": None, "required": False, "default": None, "kind": kind, "values": [], "delimiter": None, "secret": False, "help": f"{name}'s help.", **extra}

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
    assert reaches(raw["services"]["app"], "LOG") and reaches(raw["services"]["app"], "KEY") and not reaches(raw["services"]["app"], "MODE")
    assert reaches(raw["services"]["app"], "ON") and not reaches(raw["services"]["db"], "ON")
    assert refusal(schema["variables"][3], "on") and refusal(schema["variables"][3], "true") is None
    assert refusal(schema["variables"][6], "[::1]:80") is None and refusal(schema["variables"][6], "0.0.0.0") is not None
    assert interpolate("${A:-x}/${B-y}/${C:+z}/$D/$$E/${F:?m}", {"A": "", "B": "", "C": "1", "D": "d"}) == "x//z/d/$E/"

    assert env_line("K", "pa$word") == "K='pa$word'" and parse_env("K='pa$word'") == {"K": "pa$word"}
    assert env_line("K", "it's") is None
    assert set_line('# DB="x"\nKEY=""\nKEY=2\nOTHER=1', "KEY", "KEY='v'") == '# DB="x"\nKEY=""\nKEY=\'v\'\nOTHER=1\n', "the last line is the one that counts"
    assert set_line("A=1\n", "B", "B='2'") == "A=1\nB='2'\n"

    import tempfile

    with tempfile.TemporaryDirectory() as directory:
        env_file = Path(directory) / ".env"
        env_file.write_text("KEEP=1\nTOKEN=\n")
        gaps = Report(missing=[Gap("TOKEN", "probe", ""), Gap("DB", "probe", "", secret=True), Gap("KEY", "probe", "", fillable=False)])
        answers = iter(["", "it's", "t$x"])
        secrets = iter(["s"])
        skipped = fill(gaps, env_file, [schema], ask=lambda _: next(answers), ask_secret=lambda _: next(secrets), say=lambda _: None)
        assert env_file.read_text() == "KEEP=1\nTOKEN='t$x'\nDB='s'\n", env_file.read_text()
        assert env_file.stat().st_mode & 0o777 == 0o600, "a file holding a secret is its owner's alone"
        assert len(skipped) == 1 and skipped[0].startswith("KEY:"), skipped

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
    check.add_argument("--fill", action="store_true", help="ask for each missing value and write it to the env file")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        print("env_schema self-test: ok")
        if args.command is None:
            return 0
    schemas = load_schemas()
    compose = read_compose()

    if args.command == "example":
        rendered = render_example(schemas, compose, HEAD.read_text())
        if args.check:
            if EXAMPLE.read_text() != rendered:
                print("example.env is not what `make env-schema` writes from infra/env/*.json and the compose files.")
                print("Change the variable where it is declared (its Config, or the compose file), then run `make env-schema`.")
                return 1
            print("example.env matches its declarations.")
            return 0
        EXAMPLE.write_text(rendered)
        return 0

    env_file: Path = args.env_file
    env_text = env_file.read_text() if env_file.exists() else ""
    if args.command == "list":
        sys.stdout.write(render_list(schemas, compose, {**parse_env(env_text), **os.environ}))
        return 0
    if args.command == "preflight":
        report = run_preflight(schemas, compose, env_file)
        print_report(report, env_file)
        if args.fill and (report.missing or report.invalid):
            for line in fill(report, env_file, schemas):
                print(f"not filled: {line}")
            print()
            report = run_preflight(schemas, compose, env_file)
            print_report(report, env_file)
        return 1 if report.missing or report.invalid or report.errors else 0
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
