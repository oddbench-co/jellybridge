#!/bin/bash
# One-time wiring of the website into the ip-api deploy process (run by hand once, by the operator):
#   1. GitHub: let Actions open the "Deploy <sha>" pull requests
#   2. ArgoCD: create the application for web/deploy/k8s (the repository is public: no credentials)
#   3. copy the ghcr-secret pull secret into the new namespace
#   4. merge the open deploy PR (pins the image sha) and sync again
# Afterwards: Cloudflare DNS for jellybridge.oddbench.co (see ../../README.md).
#
# This repository is public, so the infrastructure's addresses come from the environment:
#   ARGOCD_URL       ArgoCD API base URL (required)
#   ARGOCD_PASSWORD  ArgoCD admin password (required)
#   ARGOCD_DEST      Kubernetes API URL of the target cluster, as registered in ArgoCD (required)
#   KUBE_CONTEXT     kubectl context of the target cluster [ip-api-prod]
set -euo pipefail
: "${ARGOCD_URL:?set ARGOCD_URL}" "${ARGOCD_PASSWORD:?set ARGOCD_PASSWORD}" "${ARGOCD_DEST:?set ARGOCD_DEST}"
export ARGOCD_PASSWORD
REPO=oddbench-co/jellybridge
APP=jellybridge-site
NS=jellybridge
CTX="${KUBE_CONTEXT:-ip-api-prod}"

echo "1/4 GitHub"
gh api -X PUT "/repos/$REPO/actions/permissions/workflow" --silent \
  --input <(echo '{"default_workflow_permissions":"read","can_approve_pull_request_reviews":true}')

echo "2/4 ArgoCD"
TOKEN=$(curl -sk -m 15 -X POST "$ARGOCD_URL/api/v1/session" -H 'Content-Type: application/json' \
  -d "$(python3 -c 'import json,os;print(json.dumps({"username":"admin","password":os.environ["ARGOCD_PASSWORD"]}))')" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')
auth=(-H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json')
curl -sk -m 60 -X POST "$ARGOCD_URL/api/v1/applications?upsert=true" "${auth[@]}" -d '{
 "metadata":{"name":"'"$APP"'","namespace":"argocd"},
 "spec":{"project":"default",
  "source":{"repoURL":"https://github.com/'"$REPO"'.git","path":"web/deploy/k8s","targetRevision":"main"},
  "destination":{"server":"'"$ARGOCD_DEST"'","namespace":"'"$NS"'"},
  "syncPolicy":{"syncOptions":["CreateNamespace=true"]}}}' \
  | python3 -c 'import sys,json; r=json.load(sys.stdin); print("app:", r.get("metadata",{}).get("name"), r.get("error",""))'
sync() { curl -sk -m 120 -X POST "$ARGOCD_URL/api/v1/applications/$APP/sync" "${auth[@]}" -d '{}' \
  | python3 -c 'import sys,json; r=json.load(sys.stdin); print("sync:", r.get("status",{}).get("sync",{}).get("status"), r.get("error",""))'; }
sync

echo "3/4 pull secret"
kubectl --context "$CTX" -n default get secret ghcr-secret -o yaml \
  | sed "s/namespace: default/namespace: $NS/" | kubectl --context "$CTX" -n "$NS" apply -f -

echo "4/4 pin the image and sync"
PR=$(gh pr list -R "$REPO" --label deploy --json number --jq '.[0].number' || true)
if [ -n "$PR" ]; then gh pr merge -R "$REPO" "$PR" --merge --delete-branch; sleep 10; sync; fi
kubectl --context "$CTX" -n "$NS" rollout status "deploy/$APP" --timeout=300s
kubectl --context "$CTX" -n "$NS" get pod,certificate 2>/dev/null || true
echo "done. Next: Cloudflare DNS jellybridge.oddbench.co (proxied), then Search Console and Ahrefs."
