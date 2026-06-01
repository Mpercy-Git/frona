use crate::Result;
use crate::aria::axtree::extract_ax_tree;
use crate::aria::diff::diff_snapshots;
use crate::aria::render::{RenderMode, render_aria_tree};
use crate::connection::BrowserConnection;
use crate::types::Snapshot;

impl BrowserConnection {
    /// `incremental` returns a unified diff against the previous snapshot on this connection.
    /// `compact` keeps only lines with `[index=` or `: <value>` plus their ancestor chain.
    pub async fn snapshot(&self, incremental: bool, compact: bool) -> Result<Snapshot> {
        let page = self.active_page().await?;
        let ax = extract_ax_tree(&page).await?;
        let mode = if compact {
            RenderMode::Compact
        } else {
            RenderMode::Ai
        };
        let rendered = render_aria_tree(&ax.root, mode, None);
        let interactive_count = ax.refs.len();
        self.store_snapshot_refs(ax.refs);

        let output = if incremental
            && let Some(prev) = self.take_last_snapshot()
        {
            diff_snapshots(&prev, &rendered)
        } else {
            rendered.clone()
        };

        self.store_last_snapshot(rendered);

        Ok(Snapshot {
            tree: output,
            interactive_count,
        })
    }
}
