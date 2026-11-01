//! The encrypted deployment files, `deploy/<role>/<app>.sops.env` (#613,
//! ENV1 #614), and the one program that opens them, `sops`.
//!
//! Every value is a secret (#613, principle 1), so envctl never holds one
//! anywhere but in memory: [`decrypt`] reads `sops`' stdout, [`encrypt`]
//! writes plain text only to `sops`' stdin, and nothing here prints a value.
//! What a file sets is readable without its key ([`names`]): sops' dotenv
//! form keeps each name in the clear and encrypts only its value.

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use std::{
	io::Write as _,
	os::unix::fs::PermissionsExt as _,
	path::{Path, PathBuf},
	process::{Command, Stdio},
};

/// The deployment files' directory, under the checkout.
pub(crate) const DEPLOY: &str = "deploy";

/// The file of compose's own `${...}` variables in each role's directory.
pub(crate) const COMPOSE: &str = "compose";

/// The suffix every deployment file carries, which `.sops.yaml`'s rule
/// matches.
pub(crate) const SUFFIX: &str = ".sops.env";

/// What a deployment file sets, in its order: sops' dotenv, each value
/// whole.
pub(crate) type Plain = IndexMap<String, String>;

/// `deploy/<role>/<app>.sops.env`, relative to the checkout.
pub(crate) fn relative(role: &str, app: &str) -> PathBuf {
	Path::new(DEPLOY).join(role).join(format!("{app}{SUFFIX}"))
}

/// The command that runs sops, its words in order: `$SOPS` (split on
/// whitespace) when it is set; else `sops` on `PATH`; else, on a machine with
/// nix but no sops (the box), `nix shell nixpkgs#sops -c sops`; else `sops`,
/// whose absence is then named.
pub(crate) fn program(shell: &IndexMap<String, String>) -> Vec<String> {
	if let Some(given) = shell.get("SOPS").filter(|given| !given.trim().is_empty()) {
		return given.split_whitespace().map(str::to_owned).collect();
	}
	let on_path = |name: &str| {
		shell.get("PATH").is_some_and(|path| {
			path
				.split(':')
				.filter(|dir| !dir.is_empty())
				.any(|dir| std::fs::metadata(Path::new(dir).join(name)).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0))
		})
	};
	if !on_path("sops") && on_path("nix") {
		return ["nix", "--extra-experimental-features", "nix-command flakes", "shell", "nixpkgs#sops", "-c", "sops"]
			.into_iter()
			.map(str::to_owned)
			.collect();
	}
	vec!["sops".to_owned()]
}

/// A process for `sops`, as [`program`] gives it. One run through nix gets
/// no `LD_LIBRARY_PATH`: a dev shell's (the `.#rust` one points it at its own
/// OpenSSL) breaks the `nix` binary, which links a newer one.
fn command(sops: &[String]) -> Command {
	let (program, args) = sops.split_first().map_or(("sops", &[][..]), |(program, args)| (program.as_str(), args));
	let mut command = Command::new(program);
	command.args(args);
	if program == "nix" {
		command.env_remove("LD_LIBRARY_PATH");
	}
	command
}

/// The names an encrypted file sets, read without decrypting it: every
/// `NAME=` line but sops' own `sops_*` metadata.
pub(crate) fn names(encrypted: &str) -> Vec<String> {
	encrypted
		.lines()
		.filter(|line| !line.trim_start().starts_with('#'))
		.filter_map(|line| line.split_once('='))
		.map(|(name, _)| name.trim().to_owned())
		// Only what is a name: other text before `=` (a sentence, a stray
		// plain line) could hold a value, and every caller prints names.
		.filter(|name| is_name(name) && !name.starts_with("sops_"))
		.collect()
}

/// Whether `encrypted` reads as a file sops wrote: a MAC, and every value
/// but sops' own wrapped in `ENC[...]`, or empty (sops leaves an empty
/// value as it is: there is nothing to hide). A name whose value is not is
/// named; a line that is no `NAME=value` by its number, never its text.
pub(crate) fn plain_values(encrypted: &str) -> Result<(), Vec<String>> {
	let mut plain = Vec::new();
	let mut mac = false;
	for (number, line) in encrypted.lines().enumerate() {
		if line.trim().is_empty() || line.trim_start().starts_with('#') {
			continue;
		}
		let Some((name, value)) = line.split_once('=') else {
			plain.push(format!("line {}", number + 1));
			continue;
		};
		if name == "sops_mac" {
			mac = true;
		} else if !name.starts_with("sops_") && !value.is_empty() && !value.starts_with("ENC[") {
			// A name is printed only when it is one; anything else on the
			// line could be a value, and is reported by number.
			plain.push(if is_name(name) { name.to_owned() } else { format!("line {}", number + 1) });
		}
	}
	if !mac {
		plain.insert(0, "sops_mac".to_owned());
	}
	if plain.is_empty() {
		Ok(())
	} else {
		Err(plain)
	}
}

/// `file` without its empty values. An empty value is how a deployment file
/// says "decided: unset" (#616), since sops' dotenv has no other way to, so
/// a binary is given it unset, never set to nothing.
pub(crate) fn set_only(mut file: Plain) -> Plain {
	file.retain(|_, value| !value.is_empty());
	file
}

/// Whether `text` is a variable name (`[A-Za-z_][A-Za-z0-9_]*`), and so safe
/// to print.
pub(crate) fn is_name(text: &str) -> bool {
	text
		.bytes()
		.enumerate()
		.all(|(at, byte)| byte == b'_' || byte.is_ascii_alphabetic() || (at > 0 && byte.is_ascii_digit()))
		&& !text.is_empty()
}

/// sops' dotenv as it decrypts it: `NAME=value` up to the line's end, with
/// `\n` standing for a newline; no quoting, no comment after a value.
pub(crate) fn parse(text: &str) -> Plain {
	text
		.lines()
		.filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
		.filter_map(|line| line.split_once('='))
		.map(|(name, value)| (name.to_owned(), value.replace("\\n", "\n")))
		.collect()
}

/// `entries` as sops' dotenv, for `sops encrypt` to read. A value sops
/// cannot give back as it was (a `\n` that is not a newline) is refused by
/// name.
pub(crate) fn render(entries: &Plain) -> Result<String> {
	let mut out = String::new();
	for (name, value) in entries {
		if !name
			.bytes()
			.enumerate()
			.all(|(at, byte)| byte == b'_' || byte.is_ascii_uppercase() || (at > 0 && byte.is_ascii_digit()))
		{
			bail!("{name} is not a variable name sops' dotenv keeps");
		}
		if value.contains("\\n") {
			bail!("{name}'s value holds a backslash and an n, which sops' dotenv reads back as a newline");
		}
		out.push_str(name);
		out.push('=');
		out.push_str(&value.replace('\n', "\\n"));
		out.push('\n');
	}
	Ok(out)
}

/// `file` decrypted into memory, run from `repo` so sops finds
/// `.sops.yaml` and its key as it would from a shell there.
pub(crate) fn decrypt(sops: &[String], repo: &Path, file: &Path) -> Result<Plain> {
	let shown = sops.join(" ");
	let output = command(sops)
		.args(["decrypt", "--input-type", "dotenv", "--output-type", "dotenv"])
		.arg(file)
		.current_dir(repo)
		.stdin(Stdio::null())
		.stderr(Stdio::piped())
		.stdout(Stdio::piped())
		.output()
		.with_context(|| format!("running {shown} (is it installed? `nix shell nixpkgs#sops`)"))?;
	if !output.status.success() {
		// sops' stderr names the file and the key it lacks, never a value.
		bail!("{shown} could not decrypt {}: {}", file.display(), String::from_utf8_lossy(&output.stderr).trim());
	}
	let text = String::from_utf8(output.stdout).with_context(|| format!("{} decrypts to text that is not UTF-8", file.display()))?;
	Ok(parse(&text))
}

/// `entries` encrypted by sops for `target` (repo-relative, so
/// `.sops.yaml`'s rule for it picks the recipients), plain text only on
/// sops' stdin. Returns the encrypted file; nothing is written here.
pub(crate) fn encrypt(sops: &[String], repo: &Path, target: &Path, entries: &Plain) -> Result<Vec<u8>> {
	let shown = sops.join(" ");
	// A first line that is no value, so a file with none (compose sets all
	// of an app's) is still one sops will encrypt; sops encrypts it too.
	let plain = format!("# {}: `sops edit` it; README.md, \"Deploying the stack\"\n{}", target.display(), render(entries)?);
	let mut child = command(sops)
		.args(["encrypt", "--input-type", "dotenv", "--output-type", "dotenv", "--filename-override"])
		.arg(target)
		.current_dir(repo)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.with_context(|| format!("running {shown} (is it installed? `nix shell nixpkgs#sops`)"))?;
	{
		let Some(mut stdin) = child.stdin.take() else {
			bail!("{shown} took no stdin");
		};
		stdin.write_all(plain.as_bytes()).with_context(|| format!("writing to {shown}"))?;
	}
	drop(plain);
	let output = child.wait_with_output().with_context(|| format!("waiting for {shown}"))?;
	if !output.status.success() {
		bail!("{shown} could not encrypt {}: {}", target.display(), String::from_utf8_lossy(&output.stderr).trim());
	}
	let written = String::from_utf8_lossy(&output.stdout);
	if let Err(plain) = plain_values(&written) {
		bail!("{shown} wrote {} with values in the clear ({}); nothing was written", target.display(), plain.join(", "));
	}
	Ok(output.stdout)
}

#[cfg(test)]
mod tests {
	use super::{names, parse, plain_values, program, render, set_only, Plain};
	use indexmap::IndexMap;
	use std::os::unix::fs::PermissionsExt as _;

	#[test]
	fn an_empty_value_is_delivered_unset() {
		let file = Plain::from([("ON".to_owned(), "true".to_owned()), ("URL".to_owned(), String::new())]);
		assert_eq!(set_only(file), Plain::from([("ON".to_owned(), "true".to_owned())]));
	}

	#[test]
	fn sops_comes_from_sops_then_nix_then_what_the_shell_says() {
		let dir = std::env::temp_dir().join(format!("envctl-program-{}", std::process::id()));
		let (with_nix, with_sops) = (dir.join("nix-only"), dir.join("both"));
		for (bin, names) in [(&with_nix, &["nix"][..]), (&with_sops, &["nix", "sops"][..])] {
			std::fs::create_dir_all(bin).unwrap();
			for name in names {
				std::fs::write(bin.join(name), "").unwrap();
				std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
			}
		}
		let shell =
			|pairs: &[(&str, &std::path::Path)]| -> IndexMap<String, String> { pairs.iter().map(|(name, value)| ((*name).to_owned(), value.display().to_string())).collect() };
		assert_eq!(program(&shell(&[("PATH", &with_sops)])), ["sops"]);
		let through_nix = program(&shell(&[("PATH", &with_nix)]));
		assert_eq!(through_nix.first().map(String::as_str), Some("nix"));
		assert_eq!(through_nix[through_nix.len() - 3..], ["nixpkgs#sops", "-c", "sops"]);
		let given = IndexMap::from([("SOPS".to_owned(), "/opt/sops --verbose".to_owned())]);
		assert_eq!(program(&given), ["/opt/sops", "--verbose"]);
		assert_eq!(program(&IndexMap::new()), ["sops"], "nothing found: sops, whose absence is then named");
		std::fs::remove_dir_all(&dir).unwrap();
	}

	const ENCRYPTED: &str = "WEBAUTHN_RP_ID=ENC[AES256_GCM,data:abc,iv:x,tag:y,type:str]\n#ENC[AES256_GCM,data:comment,type:comment]\nNUDGE_ENABLED=ENC[AES256_GCM,data:t,iv:x,tag:y,type:str]\nsops_age__list_0__map_recipient=age1xyz\nsops_mac=ENC[AES256_GCM,data:m,type:str]\nsops_version=3.11.0\n";

	#[test]
	fn names_read_without_the_key_and_metadata_is_not_a_name() {
		assert_eq!(names(ENCRYPTED), ["WEBAUTHN_RP_ID", "NUDGE_ENABLED"]);
		assert_eq!(names("pass word hunter2=old\n  # x=y\nA=ENC[v]\n"), ["A"], "what is not a name is never one");
		assert!(plain_values(ENCRYPTED).is_ok());
		assert_eq!(plain_values("A=ENC[x]\nB=plain\nsops_mac=ENC[m]\n"), Err(vec!["B".to_owned()]));
		assert!(plain_values("EMPTY=\nsops_mac=ENC[m]\n").is_ok(), "sops leaves an empty value as it is");
		assert_eq!(
			plain_values("pass word hunter2=old\nsops_mac=ENC[m]\n"),
			Err(vec!["line 1".to_owned()]),
			"what is not a name is never printed"
		);
		assert_eq!(
			plain_values("A=ENC[x]\n-----BEGIN PRIVATE KEY-----\nsops_mac=ENC[m]\n"),
			Err(vec!["line 2".to_owned()]),
			"a line's text is never shown"
		);
		assert_eq!(plain_values("A=ENC[x]\n"), Err(vec!["sops_mac".to_owned()]), "a file sops did not write");
	}

	#[test]
	fn sops_dotenv_round_trips_a_newline_and_refuses_what_it_cannot() {
		let entries: Plain = [("PEM", "a\nb"), ("URL", "https://x/?a=b#c 'q' $D")]
			.into_iter()
			.map(|(name, value)| (name.to_owned(), value.to_owned()))
			.collect();
		let text = render(&entries).unwrap();
		assert_eq!(text, "PEM=a\\nb\nURL=https://x/?a=b#c 'q' $D\n");
		assert_eq!(parse(&text), entries);
		let backslash_n = Plain::from([("A".to_owned(), "x\\ny".to_owned())]);
		assert!(render(&backslash_n).unwrap_err().to_string().starts_with("A's value"));
		let lower = Plain::from([("a".to_owned(), String::new())]);
		assert!(render(&lower).is_err());
	}
}
