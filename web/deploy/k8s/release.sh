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
SHA="$(git rev-parse origin/main 2>/dev/null || git rev-parse HEAD)"
RUN="$(gh run list -R "$REPO" --workflow Website --commit "$SHA" --json databaseId --jq '.[0].databaseId' || true)"
[ -n "$RUN" ] && gh run watch "$RUN" -R "$REPO" --exit-status --interval 20 | tail -3
PR="$(gh pr list -R "$REPO" --head "deploy-site/$SHA" --json number --jq '.[0].number' || true)"
[ -n "$PR" ] || { echo "no deploy PR for $SHA (CI still running, or the commit did not change web/)"; exit 1; }
gh pr merge -R "$REPO" "$PR" --merge --delete-branch
TOKEN=$(curl -sk -m 15 -X POST "$ARGOCD_URL/api/v1/session" -H 'Content-Type: application/json' \
  -d "$(python3 -c 'import json,os;print(json.dumps({"username":"admin","password":os.environ["ARGOCD_PASSWORD"]}))')" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')
sleep 10
curl -sk -m 120 -X POST "$ARGOCD_URL/api/v1/applications/$APP/sync" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d '{}' \
  | python3 -c 'import sys,json; r=json.load(sys.stdin); print("sync:", r.get("status",{}).get("sync",{}).get("status"), r.get("error",""))'
kubectl --context "$CTX" -n jellybridge rollout status "deploy/$APP" --timeout=300s
