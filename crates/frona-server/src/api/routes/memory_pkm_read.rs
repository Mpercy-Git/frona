use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::memory::pkm::read::{OntologyRead, PkmGraphRead, PkmEntityRead};
use crate::memory::pkm::model::{
    KnowledgeMemory, KnowledgeEntity, KnowledgeEntityLink, LinkOrigin, EntityCategory, EntityOrigin,
    SELF_ENTITY_PATH,
};

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

const SCOPE: &str = "memory";
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/memory/pkm/status", get(status))
        .route("/api/memory/pkm/reset", post(reset))
        .route("/api/memory/pkm/graph", get(graph))
        .route("/api/memory/pkm/entity", get(entity))
        .route("/api/memory/pkm/search", get(search))
}

fn require_read<'a>(auth: &AuthUser, state: &'a AppState) -> Result<&'a crate::memory::pkm::read::PkmReadService, AppError> {
    if auth.is_pat() && !auth.has_scope(SCOPE) {
        return Err(AppError::Forbidden(format!("token lacks '{SCOPE}' scope")));
    }
    state.pkm_read.as_ref().ok_or_else(|| {
        AppError::NotFound("PKM read API is not enabled (PKM backend inactive)".into())
    })
}

fn require_pkm<'a>(auth: &AuthUser, state: &'a AppState) -> Result<&'a crate::memory::pkm::PkmService, AppError> {
    if auth.is_pat() && !auth.has_scope(SCOPE) {
        return Err(AppError::Forbidden(format!("token lacks '{SCOPE}' scope")));
    }
    state.pkm_service.as_ref().ok_or_else(|| {
        AppError::NotFound("PKM API is not enabled (PKM backend inactive)".into())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResponse {
    available: bool,
    reset: Option<ResetStatusResponse>,
}

async fn status(auth: AuthUser, State(state): State<AppState>) -> Result<Json<StatusResponse>, ApiError> {
    let pkm = require_pkm(&auth, &state)?;
    let reset = pkm.reset_status(&auth.user_id).await?.map(Into::into);
    Ok(Json(StatusResponse { available: true, reset }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetStatusResponse {
    request_id: String,
    status: crate::memory::pkm::PkmResetState,
    requested_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    error: Option<String>,
}

impl From<crate::memory::pkm::PkmResetStatus> for ResetStatusResponse {
    fn from(status: crate::memory::pkm::PkmResetStatus) -> Self {
        Self {
            request_id: status.request_id,
            status: status.state,
            requested_at: status.requested_at,
            started_at: status.started_at,
            error: status.error,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResetAcceptedResponse {
    request_id: String,
    status: crate::memory::pkm::PkmResetState,
}

async fn reset(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<ResetAcceptedResponse>), ApiError> {
    let pkm = require_pkm(&auth, &state)?.clone();
    let status = pkm.request_reset(&auth.user_id).await?;
    pkm.spawn_reset(auth.user_id, status.request_id.clone());
    Ok((StatusCode::ACCEPTED, Json(ResetAcceptedResponse {
        request_id: status.request_id,
        status: status.state,
    })))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphResponse {
    revision: String,
    self_path: Option<String>,
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    legend: Vec<LegendBranch>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphNode {
    path: String,
    name: String,
    description: String,
    origin: EntityOrigin,
    category: EntityCategory,
    types: Vec<TypeResponse>,
    display_type: Option<String>,
    color_branch: String,
    hover_attributes: Vec<AttributeResponse>,
    additional_attribute_count: usize,
    relation_stats: RelationStats,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TypeResponse {
    iri: String,
    label: String,
    ancestors: Vec<TypeAncestor>,
}

#[derive(Clone, Serialize)]
struct TypeAncestor {
    iri: String,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AttributeResponse {
    property: String,
    label: String,
    datatype: String,
    value: serde_json::Value,
}

#[derive(Default, Serialize)]
struct RelationStats {
    total: usize,
    incoming: usize,
    outgoing: usize,
    asserted: usize,
    inferred: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphEdge {
    id: String,
    #[serde(rename = "fromPath")]
    from_entity_path: String,
    #[serde(rename = "toPath")]
    to_entity_path: String,
    relation: String,
    label: String,
    origin: GraphEdgeOrigin,
    source_memory_ids: Vec<String>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum GraphEdgeOrigin {
    Asserted,
    Inferred,
    Memory,
}

impl From<LinkOrigin> for GraphEdgeOrigin {
    fn from(origin: LinkOrigin) -> Self {
        match origin {
            LinkOrigin::Asserted => Self::Asserted,
            LinkOrigin::Inferred => Self::Inferred,
        }
    }
}

#[derive(Serialize)]
struct LegendBranch {
    iri: String,
    label: String,
}

async fn graph(auth: AuthUser, State(state): State<AppState>) -> Result<Json<GraphResponse>, ApiError> {
    let read = require_read(&auth, &state)?.graph(&auth.user_id).await?;
    Ok(Json(graph_response(read)?))
}

fn graph_response(read: PkmGraphRead) -> Result<GraphResponse, AppError> {
    let ontology = read.ontology;
    let mut stats: HashMap<&str, RelationStats> = HashMap::new();
    for link in &read.links {
        bump_relation(stats.entry(&link.from_entity_path).or_default(), link.origin, true);
        bump_relation(stats.entry(&link.to_entity_path).or_default(), link.origin, false);
    }
    let self_path = read.entities.iter().any(|entity| entity.path == SELF_ENTITY_PATH)
        .then(|| SELF_ENTITY_PATH.to_string());
    let entity_paths = read.entities.iter().map(|entity| entity.path.clone()).collect::<HashSet<_>>();
    let direct_pairs = read.links.iter().map(|link| normalized_pair(&link.from_entity_path, &link.to_entity_path))
        .collect::<HashSet<_>>();
    let nodes = read.entities.into_iter().map(|entity| {
        let types = type_responses(&ontology, &entity.kinds);
        let display_type = types.iter().min_by_key(|item| std::cmp::Reverse(item.ancestors.len()))
            .map(|item| item.iri.clone());
        let color_branch = display_type.as_deref().map(|iri| ontology.top_branch(iri))
            .unwrap_or_else(|| "untyped".into());
        let all_attributes = attribute_responses(&entity, &ontology);
        let visible_attributes = all_attributes.into_iter().filter(|attribute| {
            !matches!(attribute.value, serde_json::Value::Array(_) | serde_json::Value::Object(_))
        }).collect::<Vec<_>>();
        let additional_attribute_count = visible_attributes.len().saturating_sub(3);
        let hover_attributes = visible_attributes.into_iter().take(3).collect();
        GraphNode {
            relation_stats: stats.remove(entity.path.as_str()).unwrap_or_default(),
            path: entity.path, name: entity.name, description: entity.description,
            origin: entity.origin, category: entity.category, types, display_type,
            color_branch, hover_attributes, additional_attribute_count,
        }
    }).collect::<Vec<_>>();
    let mut edges = read.links.into_iter().map(|link| GraphEdge {
        id: link.id, from_entity_path: link.from_entity_path, to_entity_path: link.to_entity_path,
        label: ontology.label(&link.relation), relation: link.relation,
        origin: link.origin.into(), source_memory_ids: link.source_memory_ids,
    }).collect::<Vec<_>>();
    edges.extend(shared_memory_edges(read.sources, &entity_paths, &direct_pairs));
    let legend = legend(&ontology, &nodes);
    let revision = crate::memory::pkm::sha256_hex(&serde_json::to_string(&(&nodes, &edges))
        .map_err(|error| AppError::Internal(format!("pkm graph revision: {error}")))?);
    Ok(GraphResponse { revision, self_path, nodes, edges, legend })
}

fn normalized_pair(left: &str, right: &str) -> (String, String) {
    if left <= right { (left.into(), right.into()) } else { (right.into(), left.into()) }
}

fn shared_memory_edges(
    sources: Vec<crate::memory::pkm::model::KnowledgeEntitySource>,
    entity_paths: &HashSet<String>,
    direct_pairs: &HashSet<(String, String)>,
) -> Vec<GraphEdge> {
    let mut entities_by_memory: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for source in sources {
        if entity_paths.contains(&source.entity_path) {
            entities_by_memory.entry(source.memory_id).or_default().insert(source.entity_path);
        }
    }
    let mut memories_by_pair: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (memory_id, entities) in entities_by_memory {
        let entities = entities.into_iter().collect::<Vec<_>>();
        for left in 0..entities.len() {
            for right in (left + 1)..entities.len() {
                let pair = normalized_pair(&entities[left], &entities[right]);
                if !direct_pairs.contains(&pair) {
                    memories_by_pair.entry(pair).or_default().insert(memory_id.clone());
                }
            }
        }
    }
    memories_by_pair.into_iter().map(|((from_entity_path, to_entity_path), memory_ids)| {
        let source_memory_ids = memory_ids.into_iter().collect::<Vec<_>>();
        let label = if source_memory_ids.len() == 1 {
            "shared memory".into()
        } else {
            format!("{} shared memories", source_memory_ids.len())
        };
        let id = format!("memory:{}", crate::memory::pkm::sha256_hex(
            &format!("{from_entity_path}\0{to_entity_path}")
        ));
        GraphEdge {
            id, from_entity_path, to_entity_path, relation: "memory:shared".into(), label,
            origin: GraphEdgeOrigin::Memory, source_memory_ids,
        }
    }).collect()
}

fn bump_relation(stats: &mut RelationStats, origin: LinkOrigin, outgoing: bool) {
    stats.total += 1;
    if outgoing { stats.outgoing += 1; } else { stats.incoming += 1; }
    match origin {
        LinkOrigin::Asserted => stats.asserted += 1,
        LinkOrigin::Inferred => stats.inferred += 1,
    }
}

#[derive(Deserialize)]
struct EntityQuery {
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntityResponse {
    entity: KnowledgeEntity,
    types: Vec<TypeResponse>,
    attributes: Vec<AttributeResponse>,
    outgoing_relations: Vec<RelationResponse>,
    incoming_relations: Vec<RelationResponse>,
    memories: Vec<KnowledgeMemory>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationResponse {
    id: String,
    #[serde(rename = "fromPath")]
    from_entity_path: String,
    #[serde(rename = "toPath")]
    to_entity_path: String,
    relation: String,
    label: String,
    origin: LinkOrigin,
    source_memory_ids: Vec<String>,
    connected_name: String,
}

async fn entity(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<EntityQuery>,
) -> Result<Json<EntityResponse>, ApiError> {
    let read = require_read(&auth, &state)?.entity(&auth.user_id, &query.path).await?;
    Ok(Json(entity_response(read)))
}

fn entity_response(read: PkmEntityRead) -> EntityResponse {
    let ontology = read.ontology;
    let names = read.entities.into_iter().map(|entity| (entity.path, entity.name)).collect::<HashMap<_, _>>();
    let outgoing_relations = read.links.iter().filter(|link| link.from_entity_path == read.entity.path)
        .cloned().map(|link| relation_response(link, &names, true, &ontology)).collect();
    let incoming_relations = read.links.into_iter().filter(|link| link.to_entity_path == read.entity.path)
        .map(|link| relation_response(link, &names, false, &ontology)).collect();
    EntityResponse {
        types: type_responses(&ontology, &read.entity.kinds),
        attributes: attribute_responses(&read.entity, &ontology),
        entity: read.entity, outgoing_relations, incoming_relations, memories: read.memories,
    }
}

fn relation_response(
    link: KnowledgeEntityLink,
    names: &HashMap<String, String>,
    outgoing: bool,
    ontology: &OntologyRead,
) -> RelationResponse {
    let connected_path = if outgoing { &link.to_entity_path } else { &link.from_entity_path };
    RelationResponse {
        connected_name: names.get(connected_path).cloned().unwrap_or_else(|| connected_path.clone()),
        id: link.id, from_entity_path: link.from_entity_path, to_entity_path: link.to_entity_path,
        label: ontology.label(&link.relation), relation: link.relation, origin: link.origin,
        source_memory_ids: link.source_memory_ids,
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Serialize)]
struct SearchResponse {
    results: Vec<SearchResult>,
}

#[derive(Serialize)]
struct SearchResult {
    path: String,
    name: String,
    description: String,
    origin: EntityOrigin,
    category: EntityCategory,
    types: Vec<String>,
    aliases: Vec<String>,
}

async fn search(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ApiError> {
    let hits = require_read(&auth, &state)?.search(&auth.user_id, &query.q).await?;
    Ok(Json(SearchResponse { results: hits.into_iter().map(|hit| SearchResult {
        path: hit.path, name: hit.name, description: hit.description,
        origin: hit.origin, category: hit.category, types: hit.kinds,
        aliases: hit.aliases.into_iter().collect(),
    }).collect() }))
}

fn attribute_responses(entity: &KnowledgeEntity, ontology: &OntologyRead) -> Vec<AttributeResponse> {
    entity.attributes.as_object().into_iter().flat_map(|attributes| {
        let mut entries = attributes.iter().collect::<Vec<_>>();
        entries.sort_by_key(|(property, _)| *property);
        entries.into_iter().map(|(property, value)| AttributeResponse {
            property: property.clone(), label: ontology.label(property),
            datatype: ontology.datatype(property).unwrap_or_else(|| json_datatype(value).into()),
            value: value.clone(),
        }).collect::<Vec<_>>()
    }).collect()
}

fn json_datatype(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn type_responses(ontology: &OntologyRead, iris: &[String]) -> Vec<TypeResponse> {
    iris.iter().map(|iri| TypeResponse {
        iri: iri.clone(),
        label: ontology.label(iri),
        ancestors: ontology.ancestors(iri).into_iter().map(|ancestor| TypeAncestor {
            iri: ancestor.iri,
            label: ancestor.label,
        }).collect(),
    }).collect()
}

fn legend(ontology: &OntologyRead, nodes: &[GraphNode]) -> Vec<LegendBranch> {
    let branches = nodes.iter().map(|node| node.color_branch.clone()).collect::<HashSet<_>>();
    let mut sorted = branches.into_iter().map(|iri| LegendBranch {
        label: if iri == "untyped" { "Untyped".into() } else { ontology.label(&iri) }, iri,
    }).collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.label.cmp(&right.label));
    sorted
}
