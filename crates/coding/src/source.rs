//! Which files a tool wants when sources have to be fetched.

/// File-name filter for [`crate::CodeHost::materialize`]. A host that already
/// has the files on disk ignores it; a remote host uses it to fetch only what
/// the tool can parse. Empty means every file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceFilter {
    /// Extensions without the dot, lowercase (`rs`, `ts`).
    pub extensions: Vec<String>,
    /// Exact file names (`Dockerfile`).
    pub filenames: Vec<String>,
}

impl SourceFilter {
    pub(crate) fn new<'a>(
        extensions: impl IntoIterator<Item = &'a str>,
        filenames: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut extensions: Vec<String> = extensions.into_iter().map(str::to_string).collect();
        extensions.sort();
        extensions.dedup();
        let mut filenames: Vec<String> = filenames.into_iter().map(str::to_string).collect();
        filenames.sort();
        filenames.dedup();
        Self {
            extensions,
            filenames,
        }
    }
}
