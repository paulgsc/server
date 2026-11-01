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

/// The program to run: `$SOPS` when it is set, else `sops` on `PATH`.
pub(crate) fn program(shell: &IndexMap<String, String>) -> String {
	shell.get("SOPS").filter(|program| !program.is_empty()).cloned().unwrap_or_else(|| "sops".to_owned())
}

/// The names an encrypted file sets, read without decrypting it: every
/// `NAME=` line but sops' own `sops_*` metadata.
pub(crate) fn names(encrypted: &str) -> Vec<String> {
	encrypted
		.lines()
		.filter(|line| !line.trim_start().starts_with('#'))
		.filter_map(|line| line.split_once('='))
		.map(|(name, _)| name.trim().to_owned())
		.filter(|name| !name.is_empty() && !name.starts_with("sops_"))
		.collect()
}

/// Whether `encrypted` reads as a file sops wrote: a MAC, and every value
/// but sops' own wrapped in `ENC[...]`. A name whose value is not is named.
pub(crate) fn plain_values(encrypted: &str) -> Result<(), Vec<String>> {
	let mut plain = Vec::new();
	let mut mac = false;
	for line in encrypted.lines().filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#')) {
		let Some((name, value)) = line.split_once('=') else {
			plain.push(line.chars().take(40).collect());
			continue;
		};
		if name == "sops_mac" {
			mac = true;
		} else if !name.starts_with("sops_") && !value.starts_with("ENC[") {
			plain.push(name.to_owned());
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
pub(crate) fn decrypt(sops: &str, repo: &Path, file: &Path) -> Result<Plain> {
	let output = Command::new(sops)
		.args(["decrypt", "--input-type", "dotenv", "--output-type", "dotenv"])
		.arg(file)
		.current_dir(repo)
		.stdin(Stdio::null())
		.stderr(Stdio::piped())
		.stdout(Stdio::piped())
		.output()
		.with_context(|| format!("running {sops} (is it installed? `nix shell nixpkgs#sops`)"))?;
	if !output.status.success() {
		// sops' stderr names the file and the key it lacks, never a value.
		bail!("{sops} could not decrypt {}: {}", file.display(), String::from_utf8_lossy(&output.stderr).trim());
	}
	let text = String::from_utf8(output.stdout).with_context(|| format!("{} decrypts to text that is not UTF-8", file.display()))?;
	Ok(parse(&text))
}

/// `entries` encrypted by sops for `target` (repo-relative, so
/// `.sops.yaml`'s rule for it picks the recipients), plain text only on
/// sops' stdin. Returns the encrypted file; nothing is written here.
pub(crate) fn encrypt(sops: &str, repo: &Path, target: &Path, entries: &Plain) -> Result<Vec<u8>> {
	// A first line that is no value, so a file with none (compose sets all
	// of an app's) is still one sops will encrypt; sops encrypts it too.
	let plain = format!("# {}: `sops edit` it; README.md, \"Deploying the stack\"\n{}", target.display(), render(entries)?);
	let mut child = Command::new(sops)
		.args(["encrypt", "--input-type", "dotenv", "--output-type", "dotenv", "--filename-override"])
		.arg(target)
		.current_dir(repo)
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.with_context(|| format!("running {sops} (is it installed? `nix shell nixpkgs#sops`)"))?;
	{
		let Some(mut stdin) = child.stdin.take() else {
			bail!("{sops} took no stdin");
		};
		stdin.write_all(plain.as_bytes()).with_context(|| format!("writing to {sops}"))?;
	}
	drop(plain);
	let output = child.wait_with_output().with_context(|| format!("waiting for {sops}"))?;
	if !output.status.success() {
		bail!("{sops} could not encrypt {}: {}", target.display(), String::from_utf8_lossy(&output.stderr).trim());
	}
	let written = String::from_utf8_lossy(&output.stdout);
	if let Err(plain) = plain_values(&written) {
		bail!("{sops} wrote {} with values in the clear ({}); nothing was written", target.display(), plain.join(", "));
	}
	Ok(output.stdout)
}

#[cfg(test)]
mod tests {
	use super::{names, parse, plain_values, render, Plain};

	const ENCRYPTED: &str = "WEBAUTHN_RP_ID=ENC[AES256_GCM,data:abc,iv:x,tag:y,type:str]\n#ENC[AES256_GCM,data:comment,type:comment]\nNUDGE_ENABLED=ENC[AES256_GCM,data:t,iv:x,tag:y,type:str]\nsops_age__list_0__map_recipient=age1xyz\nsops_mac=ENC[AES256_GCM,data:m,type:str]\nsops_version=3.11.0\n";

	#[test]
	fn names_read_without_the_key_and_metadata_is_not_a_name() {
		assert_eq!(names(ENCRYPTED), ["WEBAUTHN_RP_ID", "NUDGE_ENABLED"]);
		assert!(plain_values(ENCRYPTED).is_ok());
		assert_eq!(plain_values("A=ENC[x]\nB=plain\nsops_mac=ENC[m]\n"), Err(vec!["B".to_owned()]));
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
