#!/usr/bin/env python3
"""Fill an xherd database with fake agents for screenshots (see scripts/demo.sh).

usage: demo-seed.py <data root> <workspace>
Only touches that database and a few files under <workspace>.
"""
import json, os, sqlite3, sys, time, uuid

root, ws = sys.argv[1], sys.argv[2]
db = sqlite3.connect(os.path.join(root, "xherd.db"))
now = int(time.time() * 1000)
M = 60000
sha = "0" * 64
stages = ["explore", "spec", "design", "tasks", "implement", "review", "ship", "pr"]


def task(i, title, status, evidence=None):
    t = {"id": f"T{i:02}", "title": title, "status": status, "depends": [], "files": []}
    if evidence:
        t["evidence"] = evidence
    return t


def agent(key, slug, status, ago, stage, msg, **extra):
    name = f"{key}-{slug}"
    workdir = f"{ws}/worktrees/shop/{name}"
    a = {
        "id": uuid.uuid4().hex[:12], "name": name, "ticketKey": key, "status": status,
        "createdAt": now - 200 * M, "statusSince": now - ago * M, "lastActivity": now - ago * M,
        "workdir": workdir, "repo": f"{ws}/repos/shop", "base": "origin/main",
        "pack": "xherd", "flow": "feature", "stage": stage, "lastMessage": msg,
        "visitedStages": stages[: stages.index(stage) + 1], "tokensSession": 84000,
    }
    a.update(extra)
    db.execute("insert into agents values (?,?,?,?,?)",
               (a["id"], name, status, a["createdAt"], json.dumps(a)))
    os.makedirs(f"{workdir}/.xherd/spec", exist_ok=True)
    return workdir


wd = agent("SHOP-142", "coupon-codes", "parked", 6, "design", "Design ready for review",
           gate={"kind": "docs", "stage": "design", "role": "design",
                 "files": [{"path": "design.md", "sha256": sha}]})
open(f"{wd}/.xherd/spec/design.md", "w").write("""# Coupon codes

## Approach
Codes live in a `coupons` table (code, percent, expires_at, max_uses).
The cart applies at most one coupon; the discount is computed on the server.

## Open questions
- Should a coupon stack with the free-shipping threshold?
""")
agent("SHOP-151", "order-export-csv", "parked", 14, "implement", "Commit C02 proposed",
      tasks=[task(1, "CSV writer", "done", "cargo test export"),
             task(2, "Export endpoint", "done"), task(3, "Download button", "todo")],
      gate={"kind": "commit", "id": "C02", "subject": "Add CSV export endpoint",
            "files": ["src/export.rs", "src/routes.rs"], "status": "proposed", "tasks": ["T02"]})
agent("SHOP-138", "fix-cart-rounding", "watching", 40, "pr", "Draft PR open, CI passing",
      prUrl="https://github.com/acme/shop/pull/412", prState="open", prCi="passing")
agent("SHOP-155", "search-filters", "paused", 55, "tasks", "Paused with a handoff note")
agent("SHOP-120", "dark-mode", "done", 120, "pr", "PR merged",
      prUrl="https://github.com/acme/shop/pull/398", prState="merged")
db.commit()
