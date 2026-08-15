use oxigraph::store::Store;
use oxrdf::GraphName;
use reasonable::reasoner::Reasoner;

use super::*;

const FALSE_POSITIVE_RULE: &str = "rdfs-datatype-range";

pub const CLASH_RULES: &[&str] =
    &["cax-dw", "cls-nothing2", "prp-pdw", "prp-asyp", "prp-irp"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    pub rule: String,
    pub severity: String,
    pub message: String,
}

impl Diagnostic {
    pub fn is_clash(&self) -> bool {
        CLASH_RULES.contains(&self.rule.as_str())
    }
}

pub(crate) struct Reasoned {
    pub(crate) store: Store,
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl Reasoned {
    pub fn clashes(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|diagnostic| diagnostic.is_clash())
    }
}

/// Reason over `base ⊕ delta ⊕ abox` and return the queryable closure.
pub(super) fn materialize(
    base: &[Triple],
    delta: &[Triple],
    abox: &[Triple],
) -> Result<Reasoned, AppError> {
    let mut all = Vec::with_capacity(base.len() + delta.len() + abox.len());
    all.extend_from_slice(base);
    all.extend_from_slice(delta);
    all.extend_from_slice(abox);

    let mut reasoner = Reasoner::new();
    reasoner.load_triples(all);
    reasoner.reason();

    let diagnostics = reasoner
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.rule() != FALSE_POSITIVE_RULE)
        .map(|diagnostic| Diagnostic {
            rule: diagnostic.rule().to_string(),
            severity: diagnostic.severity().to_string(),
            message: diagnostic.message().to_string(),
        })
        .collect();

    let closure = reasoner.view_output();
    let store =
        Store::new().map_err(|error| AppError::Internal(format!("ontology: reasoned store: {error}")))?;
    store
        .extend(closure.iter().map(|triple| triple.clone().in_graph(GraphName::DefaultGraph)))
        .map_err(|error| AppError::Internal(format!("ontology: load closure: {error}")))?;

    Ok(Reasoned { store, diagnostics })
}

/// One completed reasoning pass over a user's graph: the materialized closure
/// (query surface) plus the source rows it was built from (needed to compute the
/// inferred-link write-back and to validate facet bounds).
pub(crate) struct ReasonPass {
    pub(crate) reasoned: Reasoned,
    pub(super) asserted_links: Vec<KnowledgeEntityLink>,
    /// What this pass reasoned under. Carried so downstream steps
    /// (`validate`, the inferred-link write-back) compact IRIs through the *same*
    /// prefix map the ABox was built with, rather than the bundled one.
    pub(super) effective_ontology: Arc<OntologyScope>,
}


impl OntologyManager {
    pub(crate) fn assertion_graph(
        &self,
        entities: &[KnowledgeEntity],
        links: &[KnowledgeEntityLink],
    ) -> Vec<Triple> {
        abox::build_abox_triples(entities, links, &self.prefixes())
    }

    /// Build the user's ABox from the DB, reason over `effective ⊕ delta ⊕ ABox`, and
    /// return the pass (the queryable closure + the source entities/links). Pure read;
    /// nothing is written.
    pub(crate) async fn reason_user(&self, user_id: &str) -> Result<ReasonPass, AppError> {
        let user = self.load(user_id).await?;
        let entities = self.repo.list_entities(user_id).await?;
        let asserted_links = self.repo.asserted_links(user_id).await?;
        let px = user.effective_ontology().prefixes();
        let abox = abox::build_abox_triples(&entities, &asserted_links, px);
        let reasoned = user.reason(&abox)?;
        let effective_ontology = user.effective_ontology().clone();
        Ok(ReasonPass { reasoned, asserted_links, effective_ontology })
    }

    /// Like [`reason_user`], but composes a **proposed layer** of uncommitted
    /// proposed edits (`proposed_edits`) and appends `extra_abox` (e.g. entities under a
    /// proposed-but-not-stamped kind). Classify and resolve reason over this during the
    /// pass; nothing is persisted (Assemble commits the adopted subset). Pure read.
    pub(crate) async fn reason_user_with_proposed(
        &self,
        user_id: &str,
        proposed_edits: &[SchemaEdit],
        extra_abox: &[Triple],
    ) -> Result<ReasonPass, AppError> {
        let user = self.load(user_id).await?;
        let entities = self.repo.list_entities(user_id).await?;
        let asserted_links = self.repo.asserted_links(user_id).await?;
        let px = user.effective_ontology().prefixes();
        let mut abox = abox::build_abox_triples(&entities, &asserted_links, px);
        abox.extend_from_slice(extra_abox);
        let composed = ComposedOntology::with_proposed(&user, px, proposed_edits, &abox)?;
        let reasoned = composed.reason(&abox)?;
        let effective_ontology = composed.effective_ontology().clone();
        Ok(ReasonPass { reasoned, asserted_links, effective_ontology })
    }

    /// The violations for a completed pass (reasoner clashes + facet bounds).
    pub(super) fn validate(&self, pass: &ReasonPass) -> Vec<Violation> {
        validation::validate(&pass.reasoned, pass.effective_ontology.prefixes())
    }

    /// Reason, rewrite inferred entity links, and return current violations.
    pub async fn materialize(&self, user_id: &str) -> Result<Vec<Violation>, AppError> {
        let pass = self.reason_user(user_id).await?;
        let inferred = abox::extract_inferred(
            &pass.reasoned.store,
            &pass.asserted_links,
            pass.effective_ontology.prefixes(),
        )?;
        self.repo.wipe_inferred_links(user_id).await?;
        self.repo.insert_inferred_links(user_id, &inferred.links).await?;
        Ok(self.validate(&pass))
    }

    pub(crate) async fn sparql(
        &self,
        user_id: &str,
        query: &str,
    ) -> Result<QueryResults<'static>, AppError> {
        let pass = self.reason_user(user_id).await?;
        sparql::query(&pass.reasoned.store, query, pass.effective_ontology.prefixes())
    }

    pub async fn entails_type(
        &self,
        user_id: &str,
        entity_path: &str,
        class: &str,
    ) -> Result<bool, AppError> {
        let pass = self.reason_user(user_id).await?;
        let query = format!(
            "ASK {{ <{}> a <{}> }}",
            individual_iri(entity_path),
            self.prefixes().expand(class),
        );
        sparql::ask(&pass.reasoned.store, &query, pass.effective_ontology.prefixes())
    }


}
