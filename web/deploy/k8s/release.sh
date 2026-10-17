#!/bin/bash
# Release the website from the latest main: wait for the "Website" run, merge the "Deploy <sha>" pull
# request CI opened, sync the ArgoCD application and wait for the rollout.
# Needs ARGOCD_URL and ARGOCD_PASSWORD (see bootstrap.sh); KUBE_CONTEXT defaults to ip-api-prod.
set -euo pipefail
: "${ARGOCD_URL:?set ARGOCD_URL}" "${ARGOCD_PASSWORD:?set ARGOCD_PASSWORD}"
export ARGOCD_PASSWORD
REPO=oddbench-co/jellybridge
APP=jellybridge-site
CTX="${KUBE_CONTEXT:-ip-api-prod}"
# Commits that only touch web/deploy/k8s build nothing, so wait for whichever "Website" run is still
# going on main, then take the newest open deploy PR rather than one named after the latest commit.
RUN="$(gh run list -R "$REPO" --workflow Website --branch main --status in_progress --json databaseId --jq '.[0].databaseId' || true)"
[ -n "$RUN" ] && gh run watch "$RUN" -R "$REPO" --exit-status --interval 20 | tail -3
PR="$(gh pr list -R "$REPO" --label deploy --state open --json number,createdAt --jq 'sort_by(.createdAt) | last | .number // empty' || true)"
if [ -n "$PR" ]; then
  gh pr merge -R "$REPO" "$PR" --merge --delete-branch
else
  echo "no open deploy PR: syncing main as it is (e.g. a change to the manifests only)"
fi
TOKEN=$(curl -sk -m 15 -X POST "$ARGOCD_URL/api/v1/session" -H 'Content-Type: application/json' \
  -d "$(python3 -c 'import json,os;print(json.dumps({"username":"admin","password":os.environ["ARGOCD_PASSWORD"]}))')" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')
sleep 10
curl -sk -m 120 -X POST "$ARGOCD_URL/api/v1/applications/$APP/sync" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{}' \
  | python3 -c 'import sys,json; r=json.load(sys.stdin); print("sync:", r.get("status",{}).get("sync",{}).get("status"), r.get("error",""))'
kubectl --context "$CTX" -n jellybridge rollout status "deploy/$APP" --timeout=300s
