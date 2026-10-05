import sys, os, time
sys.path.insert(0,"tests/lib")
from hub import Hub, register_provider
J=r"E:/AI/ideas/prts-harness-jouzu"
hub=Hub(plugins_src=J); hub.start(); register_provider(hub)
s=hub.post("/v1/sessions",{"harnessId":"jouzu","modelProviderId":"p","modelId":"deepseek-flash"},key="a")["json"]["session"]["id"]
def st(): return (hub.get(f"/v1/sessions/{s}")["json"] or {}).get("session",{})
for _ in range(50):
    if st().get("status") not in ("starting",None): break
    time.sleep(1)
print("status:", st().get("status"))
root=os.path.join(hub.dir,"agents","jouzu")
print("agents/jouzu tree (dirs):")
for r,d,f in os.walk(root):
    lvl=r[len(root):].count(os.sep)
    if lvl<=2: print("  "*lvl+os.path.basename(r)+"/", "->", [x for x in f if x.endswith('.json')][:4])
