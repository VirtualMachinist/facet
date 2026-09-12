//! G0b: the release overlay is versioned, and 0.9 gaps live there as priority
//! values, not as platform `| force`.

use hedron_ncl::{
    contract_set_id, eval_export_with_prelude, CONTRACT_SET, OVERLAY_NCL, PLATFORM_NCL,
};

fn code_lines(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

const AGENT_POD: &str = r#"
  {
    metadata = { name = "agent-pod" },
    spec = {
      automountServiceAccountToken = true,
      containers = [
        { name = "app", image = "registry.k8s.io/pause:3.10",
          securityContext = { allowPrivilegeEscalation = true } },
      ],
    },
  }
"#;

#[test]
fn contract_set_id_is_versioned_and_matches_overlay() {
    assert_eq!(CONTRACT_SET, "k8s-1.34-h3s-0.9.1");
    assert_eq!(contract_set_id().unwrap(), CONTRACT_SET);
}

#[test]
fn release_gaps_are_in_overlay_not_platform_source() {
    let platform = code_lines(PLATFORM_NCL);
    let overlay = code_lines(OVERLAY_NCL);
    for gap in [
        "automountServiceAccountToken",
        "enableServiceLinks",
        "ClusterIP",
        "NodePort",
    ] {
        assert!(
            !platform.contains(gap),
            "platform.ncl must not legislate `{gap}`"
        );
    }
    assert!(overlay.contains("automountServiceAccountToken"));
    assert!(overlay.contains("enableServiceLinks"));
    assert!(
        !overlay.contains("force"),
        "overlay must use priority, never `| force`"
    );
    assert!(platform.contains("| force"), "platform law is `| force`");
}

#[test]
fn platform_force_hardens_but_leaves_automount_alone() {
    let v = eval_export_with_prelude("t", &format!("platform.harden_pod {AGENT_POD}")).unwrap();
    // platform law wins over the agent
    assert_eq!(v["spec"]["securityContext"]["runAsNonRoot"], true);
    let sc = &v["spec"]["containers"][0]["securityContext"];
    assert_eq!(sc["allowPrivilegeEscalation"], false);
    assert_eq!(sc["runAsNonRoot"], true);
    assert_eq!(sc["capabilities"]["drop"], serde_json::json!(["ALL"]));
    // but platform does NOT own automount: the agent's value survives
    assert_eq!(v["spec"]["automountServiceAccountToken"], true);
}

#[test]
fn overlay_priority_turns_automount_off() {
    let v = eval_export_with_prelude(
        "t",
        &format!("overlay.restrict_pod (platform.harden_pod {AGENT_POD})"),
    )
    .unwrap();
    assert_eq!(v["spec"]["automountServiceAccountToken"], false);
    assert_eq!(v["spec"]["enableServiceLinks"], false);
    assert_eq!(
        v["spec"]["containers"][0]["securityContext"]["allowPrivilegeEscalation"],
        false
    );
}

#[test]
fn operator_priority_cannot_outrank_overlay() {
    let body = format!(
        "overlay.restrict_pod ({AGENT_POD} & {{ spec.automountServiceAccountToken | priority 100 = true }})"
    );
    let v = eval_export_with_prelude("t", &body).unwrap();
    assert_eq!(v["spec"]["automountServiceAccountToken"], false);
}

#[test]
fn overlay_rejects_nodeport_with_release_blame() {
    let err = eval_export_with_prelude(
        "t",
        r#"({ spec = { type = "NodePort" } } | overlay.Service)"#,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("k8s-1.34-h3s-0.9.1"), "{msg}");
    assert!(msg.contains("NodePort"), "{msg}");
    let ok = eval_export_with_prelude(
        "t",
        r#"({ spec = { type = "ClusterIP" } } | overlay.Service)"#,
    )
    .unwrap();
    assert_eq!(ok["spec"]["type"], "ClusterIP");
}

#[test]
fn overlay_rejects_unsupported_volume_kind() {
    let err = eval_export_with_prelude(
        "t",
        r#"overlay.restrict_pod { spec = { containers = [], volumes = [{ name = "d", persistentVolumeClaim = { claimName = "x" } }] } }"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("persistentVolumeClaim"), "{err}");
    let ok = eval_export_with_prelude(
        "t",
        r#"overlay.restrict_pod { spec = { containers = [], volumes = [{ name = "c", configMap = { name = "x" } }] } }"#,
    )
    .unwrap();
    assert_eq!(ok["spec"]["volumes"][0]["name"], "c");
}

#[test]
fn platform_secret_slot_refuses_plaintext() {
    let err = eval_export_with_prelude(
        "t",
        r#"({ token = "hunter2" } | { token | platform.SecretSlot })"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("secret_ref"), "{err}");
    let ok = eval_export_with_prelude(
        "t",
        r#"({ token = { secret_ref = "github" } } | { token | platform.SecretSlot })"#,
    )
    .unwrap();
    assert_eq!(ok["token"]["secret_ref"], "github");
}

#[test]
fn platform_no_secret_fields_in_extra() {
    let err = eval_export_with_prelude(
        "t",
        r#"({ api_key = "x", owner = "me" } | platform.NoSecretFields)"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("api_key"), "{err}");
    let ok = eval_export_with_prelude(
        "t",
        r#"({ owner = "me", tier = "warm" } | platform.NoSecretFields)"#,
    )
    .unwrap();
    assert_eq!(ok["owner"], "me");
}
