import sys, os, time
sys.path.insert(0,"tests/lib")
from hub import Hub, register_provider
J=r"E:/AI/ideas/prts-harness-jouzu"
hub=Hub(plugins_src=J); hub.start(); register_provider(hub)
def st(s): return (hub.get(f"/v1/sessions/{s}")["json"] or {}).get("session",{})
ids=[]
for k in "abc":
    ids.append(hub.post("/v1/sessions",{"harnessId":"jouzu","modelProviderId":"p","modelId":"deepseek-flash"},key=k)["json"]["session"]["id"])
for _ in range(60):
    ss=[st(s).get("status") for s in ids]
    if all(x not in ("starting",None) for x in ss): break
    time.sleep(1)
print("3 concurrent jouzu sessions:", ss)
