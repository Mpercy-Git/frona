export type PageOrigin = "internal" | "external";
export type PageCategory = "concept" | "playbook";
export type LinkOrigin = "asserted" | "inferred";
export type GraphEdgeOrigin = LinkOrigin | "memory";

export interface OntologyAncestor {
  iri: string;
  label: string;
}

export interface OntologyType {
  iri: string;
  label: string;
  ancestors: OntologyAncestor[];
}

export interface MemoryAttribute {
  property: string;
  label: string;
  datatype: string;
  value: unknown;
}

export interface RelationStats {
  total: number;
  incoming: number;
  outgoing: number;
  asserted: number;
  inferred: number;
}

export interface MemoryGraphNode {
  path: string;
  name: string;
  description: string;
  origin: PageOrigin;
  category: PageCategory;
  types: OntologyType[];
  displayType: string | null;
  colorBranch: string;
  hoverAttributes: MemoryAttribute[];
  additionalAttributeCount: number;
  relationStats: RelationStats;
}

export interface MemoryGraphEdge {
  id: string;
  fromPath: string;
  toPath: string;
  relation: string;
  label: string;
  origin: GraphEdgeOrigin;
  sourceMemoryIds: string[];
}

export interface MemoryGraphResponse {
  revision: string;
  selfPath: string | null;
  nodes: MemoryGraphNode[];
  edges: MemoryGraphEdge[];
  legend: Array<{ iri: string; label: string }>;
}

export interface MemoryPageRecord {
  id: string;
  path: string;
  origin: PageOrigin;
  category: PageCategory;
  kinds: string[];
  name: string;
  description: string;
  body: string;
  related_playbooks: string[];
  attributes: Record<string, unknown>;
  use_count: number;
  aliases: string[];
  rev: string | null;
  updated_at: string;
  rendered_at: string;
}

export interface PageRelation {
  id: string;
  fromPath: string;
  toPath: string;
  relation: string;
  label: string;
  origin: LinkOrigin;
  sourceMemoryIds: string[];
  connectedName: string;
}

export interface AtomicMemory {
  id: string;
  created_at: string;
  kind: string;
  episode: Record<string, unknown> | null;
  content: string;
  relations: Array<{ relation: string; to: unknown; note: string }>;
  disposition: string;
  ended_at: string | null;
  comment: string | null;
  erroneous_at: string | null;
  evidence: Array<{ strength: string; source: Record<string, Record<string, unknown>> }>;
}

export interface MemoryPageResponse {
  page: MemoryPageRecord;
  types: OntologyType[];
  attributes: MemoryAttribute[];
  outgoingRelations: PageRelation[];
  incomingRelations: PageRelation[];
  memories: AtomicMemory[];
}

export interface MemorySearchResult {
  path: string;
  name: string;
  description: string;
  origin: PageOrigin;
  category: PageCategory;
  types: string[];
  aliases: string[];
}
