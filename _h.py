import sys, os, time
sys.path.insert(0,"tests/lib")
from hub import Hub, register_provider
J=r"E:/AI/ideas/prts-harness-jouzu"
hub=Hub(plugins_src=J); hub.start(); register_provider(hub)
def st(s): return (hub.get(f"/v1/sessions/{s}")["json"] or {}).get("session",{})
# SEQUENTIAL: one, wait active, then the next
s1=hub.post("/v1/sessions",{"harnessId":"jouzu","modelProviderId":"p","modelId":"deepseek-flash"},key="a")["json"]["session"]["id"]
for _ in range(60):
    if st(s1).get("status") not in ("starting",None): break
    time.sleep(1)
print("S1:", st(s1).get("status"))
s2=hub.post("/v1/sessions",{"harnessId":"jouzu","modelProviderId":"p","modelId":"deepseek-flash"},key="b")["json"]["session"]["id"]
for _ in range(60):
    if st(s2).get("status") not in ("starting",None): break
    time.sleep(1)
print("S2 (after S1 active):", st(s2).get("status"), st(s2).get("startError"))
# where is the injected home + profile-state?
root=os.path.join(hub.dir,"agents","jouzu")
for r,d,f in os.walk(root):
    if "profile-state.json" in f: print("profile-state:", r)
