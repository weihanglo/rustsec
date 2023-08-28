//! Vulnerability report generator
//!
//! These types map directly to the JSON report generated by `cargo-audit`,
//! but also provide the core reporting functionality used in general.

use crate::{
    advisory,
    database::{Database, Query},
    map,
    platforms::target::{Arch, OS},
    vulnerability::Vulnerability,
    warning::{self, Warning},
    Lockfile, Map,
};
use serde::{Deserialize, Serialize};

/// Vulnerability report for a given lockfile
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Report {
    /// Information about the advisory database
    #[cfg(feature = "git")]
    #[cfg_attr(docsrs, doc(cfg(feature = "git")))]
    pub database: DatabaseInfo,

    /// Information about the audited lockfile
    pub lockfile: LockfileInfo,

    /// Settings used when generating report
    pub settings: Settings,

    /// Vulnerabilities detected in project
    pub vulnerabilities: VulnerabilityInfo,

    /// Warnings about dependencies (from e.g. informational advisories)
    pub warnings: WarningInfo,
}

impl Report {
    /// Generate a report for the given advisory database and lockfile
    pub fn generate(db: &Database, lockfile: &Lockfile, settings: &Settings) -> Self {
        let vulnerabilities = db
            .query_vulnerabilities(lockfile, &settings.query())
            .into_iter()
            .filter(|vuln| !settings.ignore.contains(&vuln.advisory.id))
            .collect();

        let warnings = find_warnings(db, lockfile, settings);

        Self {
            #[cfg(feature = "git")]
            database: DatabaseInfo::new(db),
            lockfile: LockfileInfo::new(lockfile),
            settings: settings.clone(),
            vulnerabilities: VulnerabilityInfo::new(vulnerabilities),
            warnings,
        }
    }
}

/// Options to use when generating the report
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Settings {
    /// CPU architecture
    pub target_arch: Option<Arch>,

    /// Operating system
    pub target_os: Option<OS>,

    /// Severity threshold to alert at
    pub severity: Option<advisory::Severity>,

    /// List of advisory IDs to ignore
    pub ignore: Vec<advisory::Id>,

    /// Types of informational advisories to generate warnings for
    pub informational_warnings: Vec<advisory::Informational>,
}

impl Settings {
    /// Get a query which corresponds to the configured report settings.
    /// Note that queries can't filter ignored advisories, so this happens in
    /// a separate pass
    pub fn query(&self) -> Query {
        let mut query = Query::crate_scope();

        if let Some(target_arch) = self.target_arch {
            query = query.target_arch(target_arch);
        }

        if let Some(target_os) = self.target_os {
            query = query.target_os(target_os);
        }

        if let Some(severity) = self.severity {
            query = query.severity(severity);
        }

        query
    }
}

/// Information about the advisory database
#[cfg(feature = "git")]
#[cfg_attr(docsrs, doc(cfg(feature = "git")))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DatabaseInfo {
    /// Number of advisories in the database
    #[serde(rename = "advisory-count")]
    pub advisory_count: usize,

    /// Git commit hash for the last commit to the database
    #[serde(rename = "last-commit")]
    pub last_commit: Option<String>,

    /// Date when the advisory database was last committed to
    #[serde(rename = "last-updated", with = "time::serde::rfc3339::option")]
    pub last_updated: Option<time::OffsetDateTime>,
}

#[cfg(feature = "git")]
impl DatabaseInfo {
    /// Create database information from the advisory db
    pub fn new(db: &Database) -> Self {
        Self {
            advisory_count: db.iter().count(),
            last_commit: db.latest_commit().map(|c| c.commit_id.to_hex()),
            last_updated: db.latest_commit().map(|c| c.timestamp),
        }
    }
}

/// Information about `Cargo.lock`
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LockfileInfo {
    /// Number of dependencies in the lock file
    #[serde(rename = "dependency-count")]
    dependency_count: usize,
}

impl LockfileInfo {
    /// Create lockfile information from the given lockfile
    pub fn new(lockfile: &Lockfile) -> Self {
        Self {
            dependency_count: lockfile.packages.len(),
        }
    }
}

/// Information about detected vulnerabilities
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct VulnerabilityInfo {
    /// Were any vulnerabilities found?
    pub found: bool,

    /// Number of vulnerabilities found
    pub count: usize,

    /// List of detected vulnerabilities
    pub list: Vec<Vulnerability>,
}

impl VulnerabilityInfo {
    /// Create new vulnerability info
    pub fn new(list: Vec<Vulnerability>) -> Self {
        Self {
            found: !list.is_empty(),
            count: list.len(),
            list,
        }
    }
}

/// Information about warnings
pub type WarningInfo = Map<warning::WarningKind, Vec<Warning>>;

/// Find warnings from the given advisory [`Database`] and [`Lockfile`]
pub fn find_warnings(db: &Database, lockfile: &Lockfile, settings: &Settings) -> WarningInfo {
    let query = settings.query().informational(true);

    let mut warnings = WarningInfo::default();

    // TODO(tarcieri): abstract `Cargo.lock` query logic between vulnerabilities/warnings
    for advisory_vuln in db.query_vulnerabilities(lockfile, &query) {
        let advisory = &advisory_vuln.advisory;

        if settings.ignore.contains(&advisory.id) {
            continue;
        }

        if settings
            .informational_warnings
            .iter()
            .any(|info| Some(info) == advisory.informational.as_ref())
        {
            let warning_kind = match advisory
                .informational
                .as_ref()
                .expect("informational advisory")
                .warning_kind()
            {
                Some(kind) => kind,
                None => continue,
            };

            let warning = Warning::new(
                warning_kind,
                &advisory_vuln.package,
                Some(advisory.clone()),
                Some(advisory_vuln.versions.clone()),
            );

            match warnings.entry(warning.kind) {
                map::Entry::Occupied(entry) => (*entry.into_mut()).push(warning),
                map::Entry::Vacant(entry) => {
                    entry.insert(vec![warning]);
                }
            }
        }
    }

    warnings
}
