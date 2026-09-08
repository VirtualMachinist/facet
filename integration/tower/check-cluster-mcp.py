#!/usr/bin/env python3
"""Exercise real h3s mTLS/RBAC through Facet MCP with Rust Turso history.

Requires explicit project paths and an existing non-system namespace. Uses the
operator's h3s CA bundle to issue a short-lived, unprivileged test identity. The
CA key is copied only to a private temporary directory and always removed.
Fixture RBAC/ConfigMap objects are removed with UID preconditions. Credentials
are removed; sanitized transcripts, canonical YAML and Turso stores are retained.
This does not run workloads or substitute for the complete M1 harness workflow.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import tempfile
import uuid


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


class Mcp:
    def __init__(self, binary, env, root, transcript):
        self.errors = (root / "mcp-stderr.log").open("a")
        self.child = subprocess.Popen([str(binary), "mcp"], cwd=root, env=env,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                      stderr=self.errors, text=True, bufsize=1)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.child.stdout, selectors.EVENT_READ)
        self.next_id = 1
        self.transcript = transcript
        self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                     "clientInfo": {"name": "hedronetes-acceptance", "version": "1"}})
        self.child.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.child.stdin.flush()

    def request(self, method, params):
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.next_id += 1
        self.child.stdin.write(json.dumps(message) + "\n")
        self.child.stdin.flush()
        require(self.selector.select(20), "MCP response timed out")
        response = json.loads(self.child.stdout.readline())
        require(response.get("id") == message["id"], "MCP response ID differs")
        self.transcript.write(json.dumps({"request": message, "response": response}) + "\n")
        self.transcript.flush()
        return response

    def call(self, name, arguments, expected_error=False):
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        result = response["result"]
        require(bool(result.get("isError", False)) == expected_error, f"unexpected MCP error state for {name}")
        document = result["structuredContent"]
        require(json.loads(result["content"][0]["text"]) == document, "MCP text and structured result differ")
        return document

    def close(self):
        try:
            self.child.stdin.close()
        except BrokenPipeError:
            pass
        try:
            self.child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.child.terminate()
            self.child.wait(timeout=5)
        self.selector.close()
        self.child.stdout.close()
        self.errors.close()
        require(self.child.returncode == 0, "MCP did not exit cleanly")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["facet", "kubectl", "admin-kubeconfig", "pki-bundle", "root"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--namespace", required=True)
    args = parser.parse_args()
    for path in [args.facet, args.kubectl, args.admin_kubeconfig, args.pki_bundle]:
        require(path.is_absolute() and path.is_file(), "tool/credential paths must be existing absolute files")
    require(args.root.is_absolute() and args.root.is_dir(), "root must be an existing project directory")
    require(args.namespace not in {"default", "kube-system", "kube-public", "kube-node-lease"}
            and args.namespace.replace("-", "").isalnum(), "use an explicit non-system project namespace")
    os.umask(0o077)
    root = Path(tempfile.mkdtemp(prefix="facet-mcp-", dir=args.root))
    credentials = root / "credentials"
    credentials.mkdir(mode=0o700)
    prefix = "facet-mcp-" + uuid.uuid4().hex[:10]
    api = f"/api/v1/namespaces/{args.namespace}"
    rbac = f"/apis/rbac.authorization.k8s.io/v1/namespaces/{args.namespace}"
    created = []
    mcp = None
    transcript = (root / "transcript.jsonl").open("w")
    summary = {"run_id": prefix, "artifact_directory": str(root), "namespace": args.namespace,
               "harness": "deterministic MCP client; no OMP/Herdr/model claim", "outcome": "failed"}

    def kubectl(*argv, value=None):
        result = subprocess.run([str(args.kubectl), "--kubeconfig", str(args.admin_kubeconfig),
                                 "--request-timeout=15s", *argv], input=None if value is None else json.dumps(value),
                                capture_output=True, text=True, timeout=20)
        require(result.returncode == 0, f"kubectl {argv[0]} failed: {result.stderr.strip()}")
        return json.loads(result.stdout)

    def create(plural, kind, fields):
        obj = kubectl("create", "--raw", f"{rbac}/{plural}", "-f", "-", value={
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": kind,
            "metadata": {"name": prefix, "namespace": args.namespace, "annotations": {"hedronetes.io/acceptance-run": prefix}}, **fields})
        created.append((f"{rbac}/{plural}/{prefix}", obj["metadata"]["uid"]))

    def openssl(*argv):
        result = subprocess.run(["openssl", *map(str, argv)], capture_output=True, timeout=20)
        require(result.returncode == 0, "OpenSSL test credential generation failed")

    try:
        kubectl("get", "--raw", f"/api/v1/namespaces/{args.namespace}")
        admin = json.loads(args.admin_kubeconfig.read_text())
        cluster = admin["clusters"][0]["cluster"]
        endpoint = cluster["server"].rstrip("/")
        bundle = json.loads(args.pki_bundle.read_text())
        ca_pem = bundle["ca"]["certificate_pem"]
        require(base64.b64decode(cluster["certificate-authority-data"]).decode() == ca_pem,
                "operator kubeconfig and PKI bundle CA differ")
        with tempfile.TemporaryDirectory(prefix="issuer-", dir=credentials) as issuer:
            issuer = Path(issuer)
            (issuer / "ca.pem").write_text(ca_pem)
            (issuer / "ca.key").write_text(bundle["ca"]["private_key_pem"])
            openssl("genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", credentials / "client.key")
            openssl("req", "-new", "-key", credentials / "client.key", "-subj", f"/CN={prefix}", "-out", issuer / "client.csr")
            (issuer / "extensions").write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth\n")
            openssl("x509", "-req", "-in", issuer / "client.csr", "-CA", issuer / "ca.pem", "-CAkey", issuer / "ca.key",
                    "-set_serial", "0x" + uuid.uuid4().hex, "-days", "3", "-extfile", issuer / "extensions", "-out", credentials / "client.pem")
        del bundle
        encode = lambda value: base64.b64encode(value).decode()
        config = {"apiVersion": "v1", "kind": "Config", "current-context": "m1",
                  "clusters": [{"name": "cluster", "cluster": cluster}],
                  "users": [{"name": "facet", "user": {"client-certificate-data": encode((credentials / "client.pem").read_bytes()),
                                                          "client-key-data": encode((credentials / "client.key").read_bytes())}}],
                  "contexts": [{"name": "m1", "context": {"cluster": "cluster", "user": "facet", "namespace": args.namespace}}]}
        (credentials / "kubeconfig").write_text(json.dumps(config))
        del config
        create("roles", "Role", {"rules": [{"apiGroups": [""], "resources": ["configmaps"], "verbs": ["create", "get", "list", "delete"]}]})
        create("rolebindings", "RoleBinding", {"roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": prefix},
                                               "subjects": [{"apiGroup": "rbac.authorization.k8s.io", "kind": "User", "name": prefix}]})
        for directory in [root / "config", root / "machine"]:
            directory.mkdir()
        (root / "config/config.toml").write_text('[lattice]\nengine = "turso"\n')
        env = dict(os.environ, FACET_CONFIG_DIR=str(root / "config"), FACET_DATA_DIR=str(root / "machine"),
                   FACET_KUBECONFIG=str(credentials / "kubeconfig"), FACET_KUBE_CONTEXT="m1", FACET_ACTOR=prefix)
        for name in ["FACET_SESSION", "FACET_NO_RECORD", "HERDR_WORKSPACE_ID", "HERDR_TAB_ID", "HERDR_PANE_ID"]:
            env.pop(name, None)
        mcp = Mcp(args.facet, env, root, transcript)
        session = mcp.call("session_start", {"actor": prefix, "meta": {"acceptanceRun": prefix}})["session"]["id"]
        mcp.close()
        mcp = None
        env["FACET_SESSION"] = session
        mcp = Mcp(args.facet, env, root, transcript)
        listed = mcp.request("tools/list", {})["result"]["tools"]
        require({"request_run", "history_list", "history_get"} <= {tool["name"] for tool in listed}, "required MCP tools missing")
        items = []
        def item(name, method, path, body=None):
            http = {"method": method, "url": endpoint + path}
            if body is not None:
                http["body"] = {"type": "json", "data": json.dumps(body)}
            items.append({"info": {"name": name, "type": "http", "seq": len(items) + 1}, "http": http})
        item("create", "POST", api + "/configmaps", {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": prefix, "namespace": args.namespace}, "data": {"acceptanceRun": prefix}})
        item("read", "GET", api + "/configmaps/" + prefix)
        item("denied-secret", "GET", api + "/secrets")
        item("denied-namespace", "GET", "/api/v1/namespaces/kube-system/configmaps")
        workspace = root / "workspace.yml"
        def save():
            workspace.write_text(json.dumps({"opencollection": "1.0.0", "info": {"name": "h3s MCP acceptance"}, "bundled": True, "items": items}, indent=2))
        save()
        runs = []
        def action(index, status, expected_error=False):
            doc = mcp.call("request_run", {"path": str(workspace), "selector": f"items/{index}", "expect": [200, 201], "tag": [prefix]}, expected_error)
            require(doc["response"]["status"] == status, "unexpected h3s HTTP status")
            require(doc["lattice"]["recorded"] and doc["lattice"]["indexed"], "action did not persist in both stores")
            runs.append(doc["lattice"]["runId"])
            return json.loads(doc["response"]["body"]["content"])
        obj = action(0, 201)
        cm_path = api + "/configmaps/" + prefix
        created.append((cm_path, obj["metadata"]["uid"]))
        require(action(1, 200) == obj, "MCP read differs from create")
        action(2, 403, True)
        action(3, 403, True)
        item("delete", "DELETE", cm_path, {"apiVersion": "v1", "kind": "DeleteOptions", "preconditions": {"uid": obj["metadata"]["uid"]}})
        save()
        action(4, 200)
        created.remove((cm_path, obj["metadata"]["uid"]))
        history = mcp.call("history_list", {"path": str(workspace), "session": session})["runs"]
        require({run["id"] for run in history} == set(runs), "session history lost a run")
        require(sum(run["status"] == 403 for run in history) == 2, "denials missing from history")
        mcp.close(); mcp = None
        mcp = Mcp(args.facet, env, root, transcript)
        again = mcp.call("history_list", {"path": str(workspace), "session": session})["runs"]
        require(again == history, "MCP process reopen changed history")
        mcp.call("session_end", {"id": session})
        mcp.close(); mcp = None
        for db in [root / ".facet/lattice.db", root / "machine/lattice.db"]:
            require(db.with_suffix(db.suffix + ".engine").read_text() == "turso\n", "wrong database engine marker")
        summary.update(outcome="passed", session_id=session, run_ids=runs, configmap_uid=obj["metadata"]["uid"],
                       workspace_sha256=hashlib.sha256(workspace.read_bytes()).hexdigest(),
                       verified=["mTLS h3s ConfigMap create/read/delete through MCP", "namespace and Secret RBAC denials", "Turso action/session history", "MCP process reopen"])
    finally:
        cleanup_errors = []
        if mcp is not None:
            try:
                mcp.close()
            except Exception as error:
                cleanup_errors.append(str(error))
        # A server may commit before a response/recording failure. Discover only
        # this run's names and verify their nonce before considering cleanup.
        candidates = [api + "/configmaps/" + prefix, rbac + "/roles/" + prefix, rbac + "/rolebindings/" + prefix]
        for candidate in candidates:
            if any(path == candidate for path, _ in created):
                continue
            try:
                result = subprocess.run([str(args.kubectl), "--kubeconfig", str(args.admin_kubeconfig),
                                         "--request-timeout=15s", "get", "--raw", candidate],
                                        capture_output=True, text=True, timeout=20)
                if result.returncode == 0:
                    obj = json.loads(result.stdout)
                    owned = (obj.get("data", {}).get("acceptanceRun") == prefix if obj["kind"] == "ConfigMap"
                             else obj["metadata"].get("annotations", {}).get("hedronetes.io/acceptance-run") == prefix)
                    require(owned, "cleanup ownership mismatch")
                    created.append((candidate, obj["metadata"]["uid"]))
                elif "NotFound" not in result.stderr:
                    cleanup_errors.append("cannot inspect possible fixture after operation failure")
            except Exception as error:
                cleanup_errors.append(str(error))
        for path, uid in reversed(created):
            try:
                kubectl("delete", "--raw", path, "-f", "-", value={"apiVersion": "v1", "kind": "DeleteOptions", "preconditions": {"uid": uid}})
            except Exception as error:
                cleanup_errors.append(str(error))
        shutil.rmtree(credentials)
        transcript.close()
        summary["cleanup_errors"] = cleanup_errors
        summary["credentials_removed"] = True
        if cleanup_errors:
            summary["outcome"] = "failed"
        (root / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary, indent=2))
        require(not cleanup_errors, "fixture cleanup failed")


if __name__ == "__main__":
    main()
