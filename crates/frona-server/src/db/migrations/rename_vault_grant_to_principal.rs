//! The old `agent_id` column is intentionally left on rows after the rename
//! so a code rollback can still read them. The replacement index
//! (`idx_vault_grant_user_principal`) is declared in `setup_schema`, not here.

use frona_derive::migration;

#[migration("2026-04-09T21:00:00Z")]
fn rename_vault_grant_to_principal() -> &'static str {
    // The nested `{ Agent: {} }` shape matches SurrealValue's externally-tagged
    // encoding of unit enum variants. Writing `kind: 'agent'` as a plain string
    // would deserialize into a GrantPrincipal with the wrong discriminator and
    // queries that bind a `GrantPrincipalKind` value would fail to match.
    "UPDATE vault_grant
        SET principal = { kind: { Agent: {} }, id: agent_id }
        WHERE principal IS NONE AND agent_id IS NOT NONE;

     UPDATE vault_access_log
        SET principal = { kind: { Agent: {} }, id: agent_id }
        WHERE principal IS NONE AND agent_id IS NOT NONE;

     REMOVE INDEX IF EXISTS idx_vault_grant_user_agent ON TABLE vault_grant;"
}
