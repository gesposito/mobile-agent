//! The agent's dex jar and the version it must report.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The agent version this driver deploys: `agent-android/VERSION`, which
/// `build.sh` compiles into the agent's banner and hello.
pub const AGENT_VERSION: &str = include_str!("../../../../agent-android/VERSION").trim_ascii();

/// The name every agent build puts before its version in its hello.
pub const AGENT_NAME: &str = "mobile-agent-android";

/// Environment variable naming the jar to deploy instead of the default.
pub const JAR_VAR: &str = "MOBILE_AGENT_JAR";

/// Where `agent-android/build.sh` writes the jar in this source tree.
pub fn default_jar_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../agent-android/dist/mobile-agent-android.jar")
}

/// The jar could not be read.
#[derive(Debug)]
pub struct JarError {
    pub path: PathBuf,
    pub source: io::Error,
}

impl fmt::Display for JarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot read the agent jar at {} ({}); build it with agent-android/build.sh or \
             set {JAR_VAR} to its path",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for JarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// The agent's jar, read into memory, and the version its hello must name.
#[derive(Clone)]
pub struct AgentJar {
    bytes: Arc<[u8]>,
    version: String,
    path: Option<PathBuf>,
}

impl fmt::Debug for AgentJar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentJar")
            .field("version", &self.version)
            .field("bytes", &self.bytes.len())
            .field("path", &self.path)
            .finish()
    }
}

impl AgentJar {
    /// The jar at `$MOBILE_AGENT_JAR`, else at [`default_jar_path`], expected
    /// to report [`AGENT_VERSION`].
    pub fn locate() -> Result<AgentJar, JarError> {
        let path = std::env::var_os(JAR_VAR)
            .filter(|value| !value.is_empty())
            .map_or_else(default_jar_path, PathBuf::from);
        AgentJar::from_path(&path)
    }

    /// The jar at `path`, expected to report [`AGENT_VERSION`].
    pub fn from_path(path: &Path) -> Result<AgentJar, JarError> {
        let bytes = std::fs::read(path).map_err(|source| JarError {
            path: path.to_owned(),
            source,
        })?;
        Ok(AgentJar {
            bytes: bytes.into(),
            version: AGENT_VERSION.to_owned(),
            path: Some(path.to_owned()),
        })
    }

    /// A jar (or a bare dex, which `app_process` also loads) held in memory
    /// that reports `version`.
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>, version: &str) -> AgentJar {
        AgentJar {
            bytes: bytes.into(),
            version: version.to_owned(),
            path: None,
        }
    }

    /// The version the agent reports, such as `0.1.0`.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The build: the first 12 hex digits of an xxh3 digest of the jar's
    /// bytes. Two jars that report the same version but differ, such as the
    /// agent before and after a change to its payload, are two builds.
    pub fn build(&self) -> String {
        todo!("build identity")
    }

    /// The agent name and version its hello carries:
    /// `mobile-agent-android <version>`.
    pub fn agent_name(&self) -> String {
        format!("{AGENT_NAME} {}", self.version)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Where the jar was read from, when it came from a file.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_expected_version_is_the_agents_version_file() {
        assert_eq!(AGENT_VERSION, "0.1.0");
        let jar = AgentJar::from_bytes(vec![1, 2, 3], AGENT_VERSION);
        assert_eq!(jar.agent_name(), "mobile-agent-android 0.1.0");
        assert_eq!(jar.bytes(), &[1, 2, 3]);
        assert_eq!(jar.path(), None);
    }

    #[test]
    fn a_jars_build_is_a_digest_of_its_bytes() {
        let a = AgentJar::from_bytes(vec![1, 2, 3], AGENT_VERSION);
        let same = AgentJar::from_bytes(vec![1, 2, 3], "9.9.9");
        let other = AgentJar::from_bytes(vec![1, 2, 4], AGENT_VERSION);
        assert_eq!(a.build(), same.build(), "the version is not part of it");
        assert_ne!(a.build(), other.build());
        assert_eq!(a.build().len(), 12);
        assert!(a.build().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_missing_jar_says_how_to_build_it() {
        let err =
            AgentJar::from_path(Path::new("/nonexistent/mobile-agent-android.jar")).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("/nonexistent/mobile-agent-android.jar"),
            "{text}"
        );
        assert!(text.contains("agent-android/build.sh"), "{text}");
        assert!(text.contains(JAR_VAR), "{text}");
    }

    #[test]
    fn a_jar_file_is_read_with_the_current_version() {
        let dir = std::env::temp_dir().join(format!("android-jar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.jar");
        std::fs::write(&path, b"PK fake").unwrap();
        let jar = AgentJar::from_path(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(jar.bytes(), b"PK fake");
        assert_eq!(jar.version(), AGENT_VERSION);
        assert_eq!(jar.path(), Some(path.as_path()));
    }

    #[test]
    fn the_default_path_is_the_build_output() {
        let path = default_jar_path();
        assert!(
            path.ends_with("agent-android/dist/mobile-agent-android.jar"),
            "{}",
            path.display()
        );
    }
}
