//! Catalog maintenance tool (CI and operators only; not distributed).
//!
//! The signing private key is read from `WATT_CONNECT_CATALOG_SIGNING_KEY`
//! (hex Ed25519 seed) and never touches the repository.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use connect_catalog::validate::{Severity, has_errors, validate_catalog};
use connect_catalog::verify::{
    CatalogSignature, ENCODING_HEX, SIGNATURE_ALGORITHM, parse_trusted_keys, signing_bytes,
    verify_catalog_bytes,
};
use connect_catalog::{Catalog, ServerEntry, compare_versions};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use watt_credential::{SigningKeyMaterial, derive_public_key, generate_signing_key, sign_detached};

const KEY_ENV: &str = "WATT_CONNECT_CATALOG_SIGNING_KEY";

#[derive(Parser)]
#[command(
    name = "watt-connect-catalog",
    about = "Validate, build, sign and verify the watt-connect catalog"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate `<dir>/services/*.json` (and that the bundled snapshot matches).
    Validate {
        #[arg(long, default_value = "catalog")]
        dir: PathBuf,
    },
    /// Assemble the service files into one catalog snapshot.
    Build {
        #[arg(long, default_value = "catalog")]
        dir: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Snapshot version; defaults to `<UTC date>.<unix seconds>`.
        #[arg(long)]
        version: Option<String>,
    },
    /// Sign a snapshot with the key in WATT_CONNECT_CATALOG_SIGNING_KEY.
    Sign {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Refuse to sign unless the key is listed in this trusted-keys file.
        #[arg(long, default_value = "catalog/catalog.pub")]
        pubkeys: PathBuf,
    },
    /// Verify a snapshot and its signature against a trusted-keys file.
    Verify {
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        sig: PathBuf,
        #[arg(long, default_value = "catalog/catalog.pub")]
        pubkeys: PathBuf,
    },
    /// Show services added, removed or changed between two snapshots.
    Diff { old: PathBuf, new: PathBuf },
    /// Generate a signing key pair.
    Keygen,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { dir } => validate(&dir),
        Command::Build { dir, out, version } => build(&dir, &out, version),
        Command::Sign {
            catalog,
            out,
            pubkeys,
        } => sign(&catalog, &out, &pubkeys),
        Command::Verify {
            catalog,
            sig,
            pubkeys,
        } => verify(&catalog, &sig, &pubkeys),
        Command::Diff { old, new } => diff(&old, &new),
        Command::Keygen => keygen(),
    }
}

fn load_services(dir: &Path) -> Result<Vec<ServerEntry>> {
    let services_dir = dir.join("services");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&services_dir)
        .with_context(|| format!("read {}", services_dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    files.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
    files.sort();
    let mut services = Vec::new();
    for path in files {
        let entry: ServerEntry = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("{} is not a valid catalog entry", path.display()))?;
        let expected = format!("{}.json", entry.name.as_str().replace('/', "."));
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy() != expected)
        {
            bail!("{} should be named {expected}", path.display());
        }
        services.push(entry);
    }
    Ok(services)
}

fn validate(dir: &Path) -> Result<()> {
    let services = load_services(dir)?;
    let catalog = connect_catalog::bundled::assemble(services);
    let issues = validate_catalog(&catalog);
    for issue in &issues {
        println!("{issue}");
    }
    let bundled = connect_catalog::bundled::bundled_catalog();
    if bundled.services != catalog.services {
        bail!(
            "the snapshot bundled into the binary differs from {}/services; \
             update crates/connect-catalog/src/bundled.rs",
            dir.display()
        );
    }
    if has_errors(&issues) {
        bail!("catalog has errors");
    }
    let warnings = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Warning)
        .count();
    println!(
        "ok: {} services, {warnings} warnings",
        catalog.services.len()
    );
    Ok(())
}

fn build(dir: &Path, out: &Path, version: Option<String>) -> Result<()> {
    let services = load_services(dir)?;
    let mut catalog = connect_catalog::bundled::assemble(services);
    let now = chrono::Utc::now();
    catalog.version =
        version.unwrap_or_else(|| format!("{}.{}", now.format("%Y.%m.%d"), now.timestamp()));
    catalog.generated_at = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    if has_errors(&validate_catalog(&catalog)) {
        bail!("catalog has errors; run `validate`");
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, serde_json::to_vec_pretty(&catalog)?)?;
    println!("wrote {} (version {})", out.display(), catalog.version);
    Ok(())
}

fn sign(catalog_path: &Path, out: &Path, pubkeys: &Path) -> Result<()> {
    let seed = std::env::var(KEY_ENV).with_context(|| format!("{KEY_ENV} is not set"))?;
    let key = SigningKeyMaterial {
        algorithm: SIGNATURE_ALGORITHM,
        private_key_encoding: ENCODING_HEX,
        private_key: seed.trim(),
    };
    let public_key = derive_public_key(key, ENCODING_HEX).context("invalid signing key")?;
    let trusted = parse_trusted_keys(&std::fs::read_to_string(pubkeys)?);
    if !trusted.contains(&public_key) {
        bail!(
            "the signing key's public key {public_key} is not listed in {}",
            pubkeys.display()
        );
    }
    let bytes = std::fs::read(catalog_path)?;
    let catalog: Catalog = serde_json::from_slice(&bytes).context("not a catalog snapshot")?;
    if has_errors(&validate_catalog(&catalog)) {
        bail!("refusing to sign an invalid catalog");
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let signature =
        sign_detached(key, ENCODING_HEX, &signing_bytes(&value)?).context("sign catalog")?;
    let signature = CatalogSignature {
        algorithm: SIGNATURE_ALGORITHM.into(),
        public_key,
        signature,
    };
    verify_catalog_bytes(&bytes, &signature, &trusted)?;
    std::fs::write(out, serde_json::to_vec_pretty(&signature)?)?;
    println!("signed {} -> {}", catalog_path.display(), out.display());
    Ok(())
}

fn verify(catalog_path: &Path, sig: &Path, pubkeys: &Path) -> Result<()> {
    let bytes = std::fs::read(catalog_path)?;
    let signature: CatalogSignature = serde_json::from_slice(&std::fs::read(sig)?)?;
    let trusted = parse_trusted_keys(&std::fs::read_to_string(pubkeys)?);
    verify_catalog_bytes(&bytes, &signature, &trusted)?;
    let catalog: Catalog = serde_json::from_slice(&bytes)?;
    println!(
        "ok: version {} signed by {}",
        catalog.version, signature.public_key
    );
    Ok(())
}

fn diff(old: &Path, new: &Path) -> Result<()> {
    let read = |path: &Path| -> Result<Catalog> {
        Ok(serde_json::from_slice(
            &std::fs::read(path).with_context(|| format!("read {}", path.display()))?,
        )?)
    };
    let (old, new) = (read(old)?, read(new)?);
    println!("version {} -> {}", old.version, new.version);
    if compare_versions(&new.version, &old.version).is_le() {
        println!("warning: new version is not greater than the old one; clients will ignore it");
    }
    let index = |catalog: &Catalog| -> BTreeMap<String, serde_json::Value> {
        catalog
            .services
            .iter()
            .map(|entry| {
                (
                    entry.name.to_string(),
                    serde_json::to_value(entry).unwrap_or_default(),
                )
            })
            .collect()
    };
    let (old_map, new_map) = (index(&old), index(&new));
    for (name, value) in &new_map {
        match old_map.get(name) {
            None => println!("+ {name}"),
            Some(previous) if previous != value => {
                let status = |v: &serde_json::Value| {
                    v["_meta"]["watt/connect"]["status"]
                        .as_str()
                        .unwrap_or("?")
                        .to_owned()
                };
                println!(
                    "~ {name} (status {} -> {})",
                    status(previous),
                    status(value)
                );
            }
            Some(_) => {}
        }
    }
    for name in old_map.keys().filter(|name| !new_map.contains_key(*name)) {
        println!("- {name}");
    }
    Ok(())
}

fn keygen() -> Result<()> {
    let key = generate_signing_key(SIGNATURE_ALGORITHM)?;
    eprintln!("Store the private key ONLY as the CATALOG_SIGNING_KEY secret. Never commit it.");
    println!("public_key  = {}", key.public_key);
    println!("private_key = {}", key.private_key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_catalog_validates_and_round_trips_signing() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../catalog");
        validate(&root).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("catalog.json");
        build(&root, &out, Some("2099.1.1".into())).unwrap();

        let key = generate_signing_key(SIGNATURE_ALGORITHM).unwrap();
        let pubkeys = dir.path().join("catalog.pub");
        std::fs::write(&pubkeys, format!("# test\n{}\n", key.public_key)).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let signature = sign_detached(
            SigningKeyMaterial {
                algorithm: SIGNATURE_ALGORITHM,
                private_key_encoding: ENCODING_HEX,
                private_key: &key.private_key,
            },
            ENCODING_HEX,
            &signing_bytes(&value).unwrap(),
        )
        .unwrap();
        let sig = dir.path().join("catalog.sig");
        std::fs::write(
            &sig,
            serde_json::to_vec(&CatalogSignature {
                algorithm: SIGNATURE_ALGORITHM.into(),
                public_key: key.public_key,
                signature,
            })
            .unwrap(),
        )
        .unwrap();
        verify(&out, &sig, &pubkeys).unwrap();
        diff(&out, &out).unwrap();
    }
}
