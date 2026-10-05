import sys, os, time
sys.path.insert(0,"tests/lib")
from hub import Hub, register_provider
for NAME,PI,H in [("pi",r"E:/AI/ideas/prts-harness-pi","pi"),("jouzu",r"E:/AI/ideas/prts-harness-jouzu","jouzu")]:
    hub=Hub(plugins_src=PI); hub.start(); register_provider(hub)
    a=hub.post("/v1/sessions",{"harnessId":H,"modelProviderId":"p","modelId":"deepseek-flash"},key="a")
    b=hub.post("/v1/sessions",{"harnessId":H,"modelProviderId":"p","modelId":"deepseek-flash"},key="b")
    sa=a["json"]["session"]["id"]; sb=b["json"]["session"]["id"]
    def st(s): return (hub.get(f"/v1/sessions/{s}")["json"] or {}).get("session",{})
    for _ in range(50):
        A,B=st(sa),st(sb)
        if A.get("status") not in ("starting",None) and B.get("status") not in ("starting",None): break
        time.sleep(1)
    print(f"{NAME}: A={A.get('status')} B={B.get('status')}")
    hub.cleanup()
