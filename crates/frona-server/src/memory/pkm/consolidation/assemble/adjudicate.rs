//! **Adjudicate** - the schema-authoring decision core.
//!
//! The aggregate schema-authoring stage: over the pass's **proposals** (terms used
//! on classified entities but not yet declared in the TBox), the model decides each
//! (declare / align / merge / restrict / amend / defer) using `usage_impact` / `test_edit`
//! / `ontology_term_search`; code enforces these guardrails:
//!   - schema **incoherence** → hard reject (never commit),
//!   - any newly introduced existing-data **violation** → reject; the model must
//!     revise, amend, override, or defer the edit.
//!
//! This module holds the **decision core**: the per-proposal submit shape and
//! the pure gate/ladder logic ([`gate`]) - no I/O, so the ladder is unit-testable on
//! its own. The orchestration in `assemble` owns the proposals and drives the conversation
//! (`adjudicate_schema`), gates each submission (`gate_submission`), and lands the
//! result in one CAS commit (`commit`) followed by the deferred entity-stamp.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::memory::pkm::ontology::{
    AlignKind, Characteristic, EditImpact, OverrideTarget, SchemaEdit, TermKind,
};

/// One proposal to adjudicate: an undeclared term + its global usage evidence
/// (from `usage_impact`) that feeds the recurring-vs-isolated call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// The CURIE used on entities but not declared in the TBox (e.g. `frona:Database`).
    pub term: String,
    pub kind: ProposalKind,
    /// How many entities / links use the term (global - across all passes).
    pub usage_entities: usize,
    pub usage_links: usize,
    /// Model-authored intent from the stage that minted the term.
    pub description: String,
    pub proposed_edits: Vec<SchemaEdit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    Class,
    ObjectProperty,
    DataProperty,
}

pub const ADJUDICATION_BATCH_MIN: usize = 10;
pub const ADJUDICATION_BATCH_TARGET: usize = 20;
pub const ADJUDICATION_BATCH_MAX: usize = 30;

#[derive(Debug, Clone)]
pub struct AdjudicationPartition {
    pub batches: Vec<Vec<Proposal>>,
    /// Fewer than the minimum globally: accept the validated baselines without a model call.
    pub final_tail: Vec<Proposal>,
}

/// Partition editable terms from the latest proposed hierarchy. Classes establish the
/// traversal; properties attach to their declared domain/range root. Stable ordering makes
/// checkpoints and scripted tests reproducible.
pub fn partition_proposals(proposals: &[Proposal]) -> AdjudicationPartition {
    if proposals.len() < ADJUDICATION_BATCH_MIN {
        return AdjudicationPartition {
            batches: Vec::new(),
            final_tail: proposals.to_vec(),
        };
    }
    let mut parent = HashMap::<String, String>::new();
    for proposal in proposals {
        for edit in &proposal.proposed_edits {
            if let SchemaEdit::SubClassOf { sub, sup } = edit {
                parent.entry(sub.clone()).or_insert_with(|| sup.clone());
            }
        }
    }
    let root = |term: &str| {
        let mut current = term.to_string();
        let mut seen = HashSet::new();
        while seen.insert(current.clone()) {
            let Some(next) = parent.get(&current) else {
                break;
            };
            current = next.clone();
        }
        current
    };
    let depth = |term: &str| {
        let mut current = term;
        let mut seen = HashSet::new();
        let mut depth = 0usize;
        while seen.insert(current.to_string()) {
            let Some(next) = parent.get(current) else {
                break;
            };
            depth += 1;
            current = next;
        }
        depth
    };
    let mut groups = BTreeMap::<String, Vec<Proposal>>::new();
    for proposal in proposals {
        let anchor = if proposal.kind == ProposalKind::Class {
            root(&proposal.term)
        } else {
            proposal
                .proposed_edits
                .iter()
                .find_map(|edit| match edit {
                    SchemaEdit::ObjectPropertyDomain { class, .. }
                    | SchemaEdit::ObjectPropertyRange { class, .. } => Some(root(class)),
                    _ => None,
                })
                .unwrap_or_else(|| "~unanchored".into())
        };
        groups.entry(anchor).or_default().push(proposal.clone());
    }
    let mut ordered = Vec::new();
    for (_, mut group) in groups {
        group.sort_by(|a, b| {
            depth(&a.term)
                .cmp(&depth(&b.term))
                .then(a.term.cmp(&b.term))
        });
        ordered.extend(group);
    }
    let mut batches = Vec::new();
    while ordered.len() >= ADJUDICATION_BATCH_MIN {
        let remaining = ordered.len();
        let take = if remaining <= ADJUDICATION_BATCH_MAX {
            remaining
        } else if remaining - ADJUDICATION_BATCH_TARGET < ADJUDICATION_BATCH_MIN {
            remaining - ADJUDICATION_BATCH_MIN
        } else {
            ADJUDICATION_BATCH_TARGET
        };
        batches.push(ordered.drain(..take).collect());
    }
    AdjudicationPartition {
        batches,
        final_tail: ordered,
    }
}

/// The model's decision for one proposal.
/// `term` names the proposal; the tagged `decision` says what to do with it.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Adjudication {
    pub term: String,
    #[serde(flatten)]
    pub decision: Decision,
}

/// What the model decided for a proposal.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Keep the Classify-authored, already validated proposal unchanged.
    AcceptProposal,
    /// Mint the term as declared. `parent` for a class; `datatype` for a data
    /// property; `domain`/`range`/`inverse`/`characteristics` for an object property
    /// (all optional - only what applies to the term's kind).
    Declare {
        parent: Option<String>,
        datatype: Option<String>,
        domain: Option<String>,
        range: Option<String>,
        inverse: Option<String>,
        /// What the relation *means* - transitive, symmetric, and so on. Domain and
        /// range say which terms a property connects; these say how the edges behave.
        /// Object properties only; ignored for a class or data property.
        #[serde(default)]
        characteristics: Vec<Characteristic>,
    },
    /// Align a `frona:` proposal to an existing standard term (equivalence axiom +
    /// ABox re-key). `standard` is the term to align to.
    Align { standard: String },
    /// Merge this term's usage into another existing term.
    Merge { into: String },
    /// Bound a data property's values (a datatype facet).
    Restrict {
        datatype: String,
        min: Option<i64>,
        max: Option<i64>,
        pattern: Option<String>,
    },
    /// Loosen an axiom that the user's delta already committed.
    ///
    /// Retraction only, and only within the user's own delta: OWL is monotonic, so a base
    /// axiom cannot be cancelled by adding one. A genuinely
    /// too-strict base axiom stays a reversible quarantine instead.
    Amend { target: OverrideTarget },
    /// Leave the term undeclared this pass - it re-surfaces for adjudication the
    /// next time it is used.
    Defer,
}

impl ProposalKind {
    /// The case convention a repaired `frona:` term of this kind takes. Classes and
    /// properties are spelled differently by convention, and the convention is only
    /// meaningful if one function decides it.
    pub fn term_kind(self) -> TermKind {
        match self {
            ProposalKind::Class => TermKind::Class,
            ProposalKind::ObjectProperty | ProposalKind::DataProperty => TermKind::Property,
        }
    }

    fn align_kind(self) -> AlignKind {
        match self {
            ProposalKind::Class => AlignKind::Class,
            ProposalKind::ObjectProperty => AlignKind::ObjectProperty,
            ProposalKind::DataProperty => AlignKind::DataProperty,
        }
    }
}

/// A non-empty trimmed option, or `None` - the model may send `""` for "not applicable".
fn present(v: &Option<String>) -> Option<String> {
    present_str(v).map(str::to_string)
}

/// [`present`] without the allocation, for callers that only read the term.
fn present_str(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl Decision {
    /// The CURIEs *the model chose* in this decision - the ones that become axioms if it
    /// is applied, and so the ones worth checking are writable before they do.
    ///
    /// Deliberately excludes two things the model did not choose. The proposal's own `term`
    /// comes from the pass's usage, so a legacy one that is already unwritable cannot be
    /// fixed by re-submitting: the model is required to echo it verbatim. And an `Amend`
    /// target names an axiom *already committed*, quoted back from the listing - which is
    /// exactly the escape hatch for a bad one, so validating it would seal the hatch.
    pub fn proposed_terms(&self) -> Vec<&str> {
        match self {
            Decision::Declare {
                parent,
                datatype,
                domain,
                range,
                inverse,
                ..
            } => [parent, datatype, domain, range, inverse]
                .into_iter()
                .filter_map(|o| present_str(o))
                .collect(),
            Decision::Align { standard } => vec![standard.trim()],
            Decision::Merge { into } => vec![into.trim()],
            Decision::Restrict { datatype, .. } => vec![datatype.trim()],
            Decision::AcceptProposal | Decision::Amend { .. } | Decision::Defer => Vec::new(),
        }
    }

    /// The schema edits this decision implies for `term`. Empty means "changes no
    /// axioms" - `Defer` declares nothing, and `Merge` is a pure usage re-key onto an
    /// already-existing term.
    pub fn edits(&self, term: &str, kind: ProposalKind) -> Vec<SchemaEdit> {
        match self {
            Decision::Declare {
                parent,
                datatype,
                domain,
                range,
                inverse,
                characteristics,
            } => {
                let mut out = Vec::new();
                match kind {
                    // A parent is what makes a mint useful (it inherits the standard
                    // hierarchy); a bare declaration is the fallback when none is given.
                    ProposalKind::Class => match present(parent) {
                        Some(sup) => out.push(SchemaEdit::SubClassOf {
                            sub: term.into(),
                            sup,
                        }),
                        None => out.push(SchemaEdit::DeclareClass { class: term.into() }),
                    },
                    ProposalKind::ObjectProperty => {
                        out.push(SchemaEdit::DeclareObjectProperty {
                            property: term.into(),
                        });
                        if let Some(class) = present(domain) {
                            out.push(SchemaEdit::ObjectPropertyDomain {
                                property: term.into(),
                                class,
                            });
                        }
                        if let Some(class) = present(range) {
                            out.push(SchemaEdit::ObjectPropertyRange {
                                property: term.into(),
                                class,
                            });
                        }
                        if let Some(b) = present(inverse) {
                            let (a, b) = if term <= b.as_str() {
                                (term.to_string(), b)
                            } else {
                                (b, term.to_string())
                            };
                            out.push(SchemaEdit::InverseProperties { a, b });
                        }
                        // Deduped: the model repeating a characteristic is not it
                        // asserting the axiom twice, and the delta is a set anyway.
                        let mut seen = Vec::new();
                        for ch in characteristics {
                            if !seen.contains(ch) {
                                seen.push(*ch);
                                out.push(SchemaEdit::PropertyCharacteristic {
                                    property: term.into(),
                                    characteristic: *ch,
                                });
                            }
                        }
                    }
                    ProposalKind::DataProperty => {
                        out.push(SchemaEdit::DeclareDataProperty {
                            property: term.into(),
                        });
                        if let Some(datatype) = present(datatype) {
                            out.push(SchemaEdit::RestrictDatatype {
                                property: term.into(),
                                datatype,
                                min: None,
                                max: None,
                                pattern: None,
                            });
                        }
                    }
                }
                out
            }
            Decision::Align { standard } => present(&Some(standard.clone()))
                .map(|standard| {
                    vec![SchemaEdit::Align {
                        frona: term.into(),
                        standard,
                        kind: kind.align_kind(),
                    }]
                })
                .unwrap_or_default(),
            // Independent of `term` and of `kind`: an amend names the *axiom* to loosen,
            // not the proposal it was raised against. The term the model was looking at
            // when it noticed is not necessarily the term the axiom is about.
            Decision::Amend { target } => {
                vec![SchemaEdit::AmendOverride {
                    target: target.clone(),
                }]
            }
            Decision::AcceptProposal | Decision::Merge { .. } | Decision::Defer => Vec::new(),
            Decision::Restrict {
                datatype,
                min,
                max,
                pattern,
            } => {
                vec![SchemaEdit::RestrictDatatype {
                    property: term.into(),
                    datatype: datatype.trim().to_string(),
                    min: *min,
                    max: *max,
                    pattern: present(pattern),
                }]
            }
        }
    }

    /// The term this proposal's usage should be re-keyed to when entities are stamped -
    /// an alignment target or a merge survivor. `None` keeps the proposed term.
    pub fn rename_target(&self) -> Option<&str> {
        let t = match self {
            Decision::Align { standard } => standard.trim(),
            Decision::Merge { into } => into.trim(),
            _ => return None,
        };
        (!t.is_empty()).then_some(t)
    }

    /// A short label for logs and the amend feedback.
    pub fn label(&self) -> &'static str {
        match self {
            Decision::AcceptProposal => "accept_proposal",
            Decision::Declare { .. } => "declare",
            Decision::Align { .. } => "align",
            Decision::Merge { .. } => "merge",
            Decision::Restrict { .. } => "restrict",
            Decision::Amend { .. } => "amend",
            Decision::Defer => "defer",
        }
    }
}

/// The model's whole adjudicate submission: a decision per proposal.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AdjudicationResult {
    pub decisions: Vec<Adjudication>,
    #[serde(default)]
    pub amendment_nominations: Vec<AmendmentNomination>,
}

/// A read-only existing term discovered to need repair. It becomes editable only after
/// the next queue repartition.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AmendmentNomination {
    pub term: String,
    pub term_kind: ProposalKind,
    pub target: OverrideTarget,
    pub evidence: String,
}

/// The outcome for one candidate edit set, given its dry-run [`EditImpact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// The edit makes the schema unsatisfiable: reject it without committing.
    Incoherent,
    /// The edit is coherent but would invalidate existing facts. Ontology edits may not
    /// enter the accepted set while any projected A-Box fact fails validation.
    DataViolations { affected: usize },
    /// Safe to commit. No projected fact is invalidated.
    Commit { quarantine: usize },
}

/// The pure guardrail decision. Both T-Box incoherence and any projected
/// A-Box violation block the edit.
pub fn gate(impact: &EditImpact) -> GateOutcome {
    if !impact.incoherence.is_empty() {
        return GateOutcome::Incoherent;
    }
    let n = impact.data_violations.len();
    if n > 0 {
        GateOutcome::DataViolations { affected: n }
    } else {
        GateOutcome::Commit { quarantine: n }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::pkm::ontology::Violation;
    use crate::memory::pkm::ontology::ViolationSource;

    fn facet_violation(entity: &str) -> Violation {
        Violation {
            source: ViolationSource::Facet,
            rule: "xsd-range".into(),
            detail: format!("frona:port on {entity}: 99999 > max 65535"),
            subject: Some(entity.into()),
        }
    }

    #[test]
    fn incoherence_hard_blocks_regardless_of_cap() {
        let impact = EditImpact {
            incoherence: vec!["cax-dw: Person ⊥ Organization".into()],
            data_violations: vec![],
        };
        assert_eq!(gate(&impact), GateOutcome::Incoherent);
    }

    #[test]
    fn any_data_violation_rejects_the_ontology_edit() {
        let impact = EditImpact {
            incoherence: vec![],
            data_violations: vec![facet_violation("services/a")],
        };
        assert_eq!(gate(&impact), GateOutcome::DataViolations { affected: 1 });
    }

    #[test]
    fn multiple_data_violations_are_rejected() {
        let impact = EditImpact {
            incoherence: vec![],
            data_violations: (0..6)
                .map(|i| facet_violation(&format!("services/{i}")))
                .collect(),
        };
        assert_eq!(gate(&impact), GateOutcome::DataViolations { affected: 6 });
    }

    fn declare(parent: Option<&str>) -> Decision {
        Decision::Declare {
            parent: parent.map(str::to_string),
            datatype: None,
            domain: None,
            range: None,
            inverse: None,
            characteristics: Vec::new(),
        }
    }

    /// A class mint is only useful if it hangs off the standard hierarchy, so a
    /// declared parent lowers to a subclass axiom rather than a bare declaration.
    #[test]
    fn declaring_a_class_lowers_to_a_subclass_axiom_when_a_parent_is_given() {
        assert_eq!(
            declare(Some("schema:SoftwareApplication"))
                .edits("frona:Database", ProposalKind::Class),
            vec![SchemaEdit::SubClassOf {
                sub: "frona:Database".into(),
                sup: "schema:SoftwareApplication".into()
            }]
        );
        assert_eq!(
            declare(None).edits("frona:Database", ProposalKind::Class),
            vec![SchemaEdit::DeclareClass {
                class: "frona:Database".into()
            }]
        );
        // the model may send "" for "not applicable" - that is not a parent named "".
        assert_eq!(
            declare(Some("   ")).edits("frona:Database", ProposalKind::Class),
            vec![SchemaEdit::DeclareClass {
                class: "frona:Database".into()
            }]
        );
    }

    #[test]
    fn declaring_an_object_property_carries_domain_range_and_inverse() {
        let d = Decision::Declare {
            parent: None,
            datatype: None,
            domain: Some("schema:Person".into()),
            range: Some("schema:Organization".into()),
            inverse: Some("frona:employs".into()),
            characteristics: Vec::new(),
        };
        let edits = d.edits("frona:worksFor", ProposalKind::ObjectProperty);
        assert_eq!(edits.len(), 4, "{edits:?}");
        assert!(edits.contains(&SchemaEdit::DeclareObjectProperty {
            property: "frona:worksFor".into()
        }));
        assert!(edits.contains(&SchemaEdit::InverseProperties {
            a: "frona:employs".into(),
            b: "frona:worksFor".into()
        }));
    }

    /// Characteristics ride alongside domain/range/inverse: one axiom each, on top of
    /// the declaration, in the order the model gave them.
    #[test]
    fn declaring_an_object_property_carries_its_characteristics() {
        let d = Decision::Declare {
            parent: None,
            datatype: None,
            domain: None,
            range: None,
            inverse: None,
            characteristics: vec![Characteristic::Transitive, Characteristic::Irreflexive],
        };
        assert_eq!(
            d.edits("frona:partOf", ProposalKind::ObjectProperty),
            vec![
                SchemaEdit::DeclareObjectProperty {
                    property: "frona:partOf".into()
                },
                SchemaEdit::PropertyCharacteristic {
                    property: "frona:partOf".into(),
                    characteristic: Characteristic::Transitive,
                },
                SchemaEdit::PropertyCharacteristic {
                    property: "frona:partOf".into(),
                    characteristic: Characteristic::Irreflexive,
                },
            ]
        );
    }

    #[test]
    fn repeated_characteristics_collapse_to_one_axiom() {
        let d = Decision::Declare {
            parent: None,
            datatype: None,
            domain: None,
            range: None,
            inverse: None,
            characteristics: vec![
                Characteristic::Symmetric,
                Characteristic::Symmetric,
                Characteristic::Functional,
            ],
        };
        let edits = d.edits("frona:knows", ProposalKind::ObjectProperty);
        assert_eq!(
            edits.len(),
            3,
            "declaration + two distinct characteristics: {edits:?}"
        );
    }

    /// Characteristics are an object-property notion. A class or data property that
    /// carries them anyway (a model filling in every field) declares nothing extra -
    /// `owl:TransitiveProperty` on a class is not a thing the reasoner can use.
    #[test]
    fn characteristics_are_ignored_for_a_class_or_data_property() {
        let d = Decision::Declare {
            parent: None,
            datatype: None,
            domain: None,
            range: None,
            inverse: None,
            characteristics: vec![Characteristic::Transitive],
        };
        assert_eq!(
            d.edits("frona:Database", ProposalKind::Class),
            vec![SchemaEdit::DeclareClass {
                class: "frona:Database".into()
            }]
        );
        assert_eq!(
            d.edits("frona:port", ProposalKind::DataProperty),
            vec![SchemaEdit::DeclareDataProperty {
                property: "frona:port".into()
            }]
        );
    }

    /// Align is the good outcome: one equivalence axiom, and the term is renamed so
    /// entities are stamped with the standard term directly (never mint-then-realign).
    #[test]
    fn align_lowers_to_an_equivalence_and_renames_the_term() {
        let d = Decision::Align {
            standard: "schema:Organization".into(),
        };
        assert_eq!(
            d.edits("frona:Company", ProposalKind::Class),
            vec![SchemaEdit::Align {
                frona: "frona:Company".into(),
                standard: "schema:Organization".into(),
                kind: AlignKind::Class,
            }]
        );
        assert_eq!(d.rename_target(), Some("schema:Organization"));
    }

    #[test]
    fn merge_renames_without_declaring_anything() {
        let d = Decision::Merge {
            into: "frona:Database".into(),
        };
        assert!(d.edits("frona:Db", ProposalKind::Class).is_empty());
        assert_eq!(d.rename_target(), Some("frona:Database"));
    }

    /// An amend names the **axiom**, not the proposal it was raised against, so it is
    /// independent of both `term` and `kind`: the term the model was inspecting when it
    /// noticed the problem is rarely the term the offending axiom is about.
    #[test]
    fn amend_retracts_the_axiom_it_names_whatever_term_raised_it() {
        let target = OverrideTarget::Characteristic {
            property: "frona:partOf".into(),
            characteristic: Characteristic::Transitive,
        };
        let d = Decision::Amend {
            target: target.clone(),
        };
        let expected = vec![SchemaEdit::AmendOverride { target }];

        for (term, kind) in [
            ("frona:Unrelated", ProposalKind::Class),
            ("frona:alsoUnrelated", ProposalKind::ObjectProperty),
            ("frona:port", ProposalKind::DataProperty),
        ] {
            assert_eq!(
                d.edits(term, kind),
                expected,
                "independent of {term}/{kind:?}"
            );
        }
        assert_eq!(
            d.rename_target(),
            None,
            "nothing is re-keyed by loosening an axiom"
        );
        assert_eq!(d.label(), "amend");
    }

    /// All three retractable axiom kinds reach `AmendOverride` unchanged - this decision
    /// is a pass-through, and the matching logic that decides what actually goes lives in
    /// `schema::override_matches`.
    #[test]
    fn every_override_target_passes_through_to_the_schema_edit() {
        for target in [
            OverrideTarget::Disjoint {
                a: "frona:Tool".into(),
                b: "frona:Service".into(),
            },
            OverrideTarget::Facet {
                property: "frona:port".into(),
            },
            OverrideTarget::Characteristic {
                property: "frona:knows".into(),
                characteristic: Characteristic::Symmetric,
            },
        ] {
            assert_eq!(
                Decision::Amend {
                    target: target.clone()
                }
                .edits("frona:X", ProposalKind::Class),
                vec![SchemaEdit::AmendOverride { target }]
            );
        }
    }

    /// Defer declares nothing and renames nothing - the term stays in use and comes
    /// back next pass.
    #[test]
    fn defer_is_inert() {
        assert!(
            Decision::Defer
                .edits("frona:Whatever", ProposalKind::Class)
                .is_empty()
        );
        assert_eq!(Decision::Defer.rename_target(), None);
    }

    #[test]
    fn restrict_lowers_to_a_datatype_facet() {
        let d = Decision::Restrict {
            datatype: "xsd:integer".into(),
            min: Some(1),
            max: Some(65535),
            pattern: None,
        };
        assert_eq!(
            d.edits("frona:port", ProposalKind::DataProperty),
            vec![SchemaEdit::RestrictDatatype {
                property: "frona:port".into(),
                datatype: "xsd:integer".into(),
                min: Some(1),
                max: Some(65535),
                pattern: None,
            }]
        );
    }

    #[test]
    fn clean_edit_commits_with_no_quarantine() {
        let impact = EditImpact::default();
        assert_eq!(gate(&impact), GateOutcome::Commit { quarantine: 0 });
    }

    fn proposal(index: usize, parent: Option<&str>) -> Proposal {
        let term = format!("frona:Class{index}");
        let proposed_edits = parent
            .map(|sup| {
                vec![SchemaEdit::SubClassOf {
                    sub: term.clone(),
                    sup: sup.into(),
                }]
            })
            .unwrap_or_else(|| {
                vec![SchemaEdit::DeclareClass {
                    class: term.clone(),
                }]
            });
        Proposal {
            term,
            kind: ProposalKind::Class,
            usage_entities: 1,
            usage_links: 0,
            description: "test class".into(),
            proposed_edits,
        }
    }

    #[test]
    fn fewer_than_minimum_is_a_model_free_final_tail() {
        let proposals: Vec<_> = (0..9).map(|i| proposal(i, Some("schema:Thing"))).collect();
        let partition = partition_proposals(&proposals);
        assert!(partition.batches.is_empty());
        assert_eq!(partition.final_tail.len(), 9);
    }

    #[test]
    fn partition_never_leaves_a_one_term_model_batch() {
        let proposals: Vec<_> = (0..31).map(|i| proposal(i, Some("schema:Thing"))).collect();
        let partition = partition_proposals(&proposals);
        assert!(partition.final_tail.is_empty());
        assert!(partition.batches.iter().all(|batch| {
            (ADJUDICATION_BATCH_MIN..=ADJUDICATION_BATCH_MAX).contains(&batch.len())
        }));
        assert_eq!(partition.batches.iter().map(Vec::len).sum::<usize>(), 31);
    }

    #[test]
    fn parent_precedes_child_inside_a_hierarchy_partition() {
        let mut proposals: Vec<_> = (0..10).map(|i| proposal(i, Some("schema:Thing"))).collect();
        proposals[1].proposed_edits = vec![SchemaEdit::SubClassOf {
            sub: proposals[1].term.clone(),
            sup: proposals[0].term.clone(),
        }];
        let partition = partition_proposals(&proposals);
        let batch = &partition.batches[0];
        let parent = batch
            .iter()
            .position(|p| p.term == proposals[0].term)
            .unwrap();
        let child = batch
            .iter()
            .position(|p| p.term == proposals[1].term)
            .unwrap();
        assert!(parent < child, "parent must be adjudicated before child");
    }
}
