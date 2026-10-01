use async_trait::async_trait;

use crate::core::error::AppError;
use crate::core::repository::Repository;
use crate::db::repo::generic::SurrealRepo;

use super::models::CostReport;

#[async_trait]
pub trait CostReportRepository: Repository<CostReport> {
    /// Reports are instance-wide in content, so this lists every report
    /// regardless of which admin's run produced it — an operator wants the
    /// server's history, not their own. Newest first.
    async fn list_recent(&self, limit: u32) -> Result<Vec<CostReport>, AppError>;
}

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl CostReportRepository for SurrealRepo<CostReport> {
    async fn list_recent(&self, limit: u32) -> Result<Vec<CostReport>, AppError> {
        let query =
            format!("{SELECT_CLAUSE} FROM cost_report ORDER BY created_at DESC LIMIT $limit");
        let mut result = self
            .db()
            .query(&query)
            .bind(("limit", limit))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let reports: Vec<CostReport> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(reports)
    }
}
