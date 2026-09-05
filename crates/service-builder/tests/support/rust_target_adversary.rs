// Independent adversarial admission tests are added here by the assigned reviewer.

use std::collections::BTreeMap;
use std::convert::Infallible;

use ess_gen::{Artifact, Provenance};
use ess_synth::{
    Capability, CapabilityKind, Synthesis, SynthesisPlan, TargetRefusal, TargetReport,
    TargetWeakening,
};

use super::{IntoSynthesisResult, admit_rust_target};
use crate::ess::EssSources;

fn sources() -> EssSources {
    EssSources::new(BTreeMap::from([(
        "system.yaml".to_owned(),
        "format: ess/1\nsystem: adversary\nversion: v1\ndomains: [adversary.model]\ndomain: adversary.model\n".to_owned(),
    )]))
    .expect("well-formed source set")
}

fn report() -> TargetReport {
    TargetReport {
        provenance: Provenance {
            system: "adversary".to_owned(),
            specification_version: "v19".to_owned(),
            source_digest: "a".repeat(64),
            contract_digest: "b".repeat(64),
        },
        target: "rust",
        weakenings: Vec::new(),
        refusals: Vec::new(),
    }
}

fn refusal(detail: &str) -> TargetRefusal {
    TargetRefusal {
        capability: Capability {
            kind: CapabilityKind::DomainType,
            source: "adversary.model.Item".to_owned(),
        },
        detail: detail.to_owned(),
    }
}

fn synthesis(target: Option<TargetReport>) -> Synthesis {
    let ir = sources().compile().expect("empty model compiles");
    Synthesis {
        plan: SynthesisPlan::of(&ir),
        artifacts: BTreeMap::from([
            ("empty.txt".to_owned(), Artifact::new("empty.txt", "")),
            (
                "opaque.txt".to_owned(),
                Artifact::new("opaque.txt", "nul:\0 crlf:\r\n unicode: λ 🦀\n"),
            ),
        ]),
        target,
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("recursive row at model.yaml:27 (code {code})")]
struct RootCause {
    code: u32,
}

#[derive(Debug, thiserror::Error)]
#[error("rust target failure in adversary.model")]
struct NestedFailure {
    #[source]
    cause: RootCause,
}

#[test]
fn checked_failure_retains_nested_typed_source_chain() {
    let error = Err::<Synthesis, _>(NestedFailure {
        cause: RootCause { code: 731 },
    })
    .into_synthesis_result()
    .err()
    .expect("checked failure must remain an error");
    let original = error
        .downcast_ref::<NestedFailure>()
        .expect("original outer error type must survive");
    assert_eq!(original.cause, RootCause { code: 731 });
    let chain = error.chain().map(ToString::to_string).collect::<Vec<_>>();
    assert_eq!(
        chain,
        [
            "rust target failure in adversary.model",
            "recursive row at model.yaml:27 (code 731)",
        ]
    );
    assert!(error.root_cause().is::<RootCause>());
    assert!(format!("{error:#}").contains("model.yaml:27"));
}

#[test]
fn successful_result_cannot_erase_rejected_report_before_admission() {
    let mut wrong_target = report();
    wrong_target.target = "go";
    let mut refused = report();
    refused.refusals.push(refusal("unrepresentable row"));
    let mut weakened = report();
    weakened.weakenings.push(TargetWeakening {
        guarantee: "static exhaustiveness".to_owned(),
        instead: "fallible runtime check".to_owned(),
        affects: vec![CapabilityKind::EntityLifecycle],
    });
    for rejected in [wrong_target, refused, weakened] {
        let expected = rejected.clone();
        let actual = Ok::<_, Infallible>(synthesis(Some(rejected)))
            .into_synthesis_result()
            .expect("successful checked result reaches report admission");
        assert_eq!(actual.target.as_ref(), Some(&expected));
        assert!(
            admit_rust_target(actual.target.as_ref()).is_err(),
            "successful Result wrapping must not launder a rejected target report"
        );
    }
}

#[test]
fn repeated_report_notes_and_source_provenance_are_not_deduplicated() {
    let mut rejected = report();
    let detail = "duplicate occurrence marker\nsource row α | reason `β`";
    rejected.refusals = vec![refusal(detail), refusal(detail)];
    let weakening = TargetWeakening {
        guarantee: "repeated guarantee marker".to_owned(),
        instead: "repeated replacement marker".to_owned(),
        affects: vec![CapabilityKind::DomainType],
    };
    rejected.weakenings = vec![weakening.clone(), weakening];
    let before = rejected.to_canonical_json();
    let error = admit_rust_target(Some(&rejected))
        .expect_err("repeated notes are still refusals")
        .to_string();
    assert_eq!(error.matches(detail).count(), 2);
    assert_eq!(error.matches("repeated guarantee marker").count(), 2);
    assert_eq!(error.matches("repeated replacement marker").count(), 2);
    assert!(error.contains(&rejected.provenance.source_digest));
    assert!(error.contains(&rejected.provenance.contract_digest));
    assert!(error.contains("adversary v19"));
    assert_eq!(rejected.to_canonical_json(), before);
}

#[test]
fn an_empty_weakening_record_is_not_an_empty_report() {
    let mut rejected = report();
    rejected.weakenings.push(TargetWeakening {
        guarantee: String::new(),
        instead: String::new(),
        affects: Vec::new(),
    });
    let error = admit_rust_target(Some(&rejected))
        .expect_err("the policy rejects any weakening, even one with no descriptive content")
        .to_string();
    assert!(error.contains("1 weakening(s), 0 target refusal(s)"));
}

#[test]
fn both_success_forms_preserve_empty_and_non_ascii_artifact_bytes() {
    for target in [None, Some(report())] {
        let expected = synthesis(target.clone());
        let direct = synthesis(target.clone())
            .into_synthesis_result()
            .expect("direct value remains successful");
        let checked = Ok::<_, Infallible>(synthesis(target))
            .into_synthesis_result()
            .expect("successful checked value remains successful");
        for actual in [direct, checked] {
            admit_rust_target(actual.target.as_ref()).expect("successful target is admitted");
            assert_eq!(actual.plan, expected.plan);
            assert_eq!(actual.target, expected.target);
            assert_eq!(actual.artifacts, expected.artifacts);
        }
    }
}

#[test]
fn zero_capability_model_keeps_the_historical_build_ess_success_path() {
    let sources = sources();
    let ir = sources.compile().expect("empty model compiles");
    let original = ess_synth::synthesize(&ir)
        .into_synthesis_result()
        .expect("historical empty Rust synthesis succeeds");
    assert!(original.plan.capabilities.is_empty());
    let original_projections = ess_gen::generate_all(&ir).expect("empty model projects");
    let built =
        super::build_ess(&sources).expect("zero capabilities alone is not a target refusal");
    assert_eq!(built.ir.to_canonical_json(), ir.to_canonical_json());
    assert_eq!(built.plan, original.plan);
    assert_eq!(built.synthesis, original.artifacts);
    assert_eq!(built.projections, original_projections);
}
