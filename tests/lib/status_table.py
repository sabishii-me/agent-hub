"""Print a status TABLE from real observations. Table only reflects STATE.

Colors: GREEN = the observed result equals the expected result for that case; RED = it does
not. BRIGHT/normal distinguishes a mock-injected run from a real one. The table carries a
fixed footer: all-green does NOT mean the system is correct - correctness is judged across
ENVIRONMENTS (dev vs staging vs prod), and a dev run being green while prod is also green is
itself a defect (T0). A green row means ONLY "this case met its expectation here".
"""

G = "\033[32m"; R = "\033[31m"; Y = "\033[33m"; D = "\033[2m"; B = "\033[1m"; X = "\033[0m"

def table(title, env, mock, rows, note=None):
    """rows: list of (case, expected, observed, ok_bool)."""
    print(f"\n{B}{title}{X}   [{env}{'/mock' if mock else ''}]")
    if note:
        print(f"{D}{note}{X}")
    w = max(len(r[0]) for r in rows) if rows else 4
    print(f"{B}{'case'.ljust(w)} | expected | observed | result{X}")
    print("-" * (w + 40))
    for case, exp, obs, ok in rows:
        col = G if ok else R
        mark = "GREEN" if ok else "RED"
        print(f"{case.ljust(w)} | {str(exp)[:20].ljust(8)} | {str(obs)[:30].ljust(8)} | {col}{mark}{X}")
    # count
    green = sum(1 for r in rows if r[3]); red = len(rows) - green
    print(f"{B}summary: {G}{green} green{X} {B}/{X} {R}{red} red{X}")
    print(f"{D}all-green does NOT mean correct. correctness = the difference ACROSS environments "
          f"(dev/staging/prod). a green dev row + a green prod row for the same case is a T0 issue.{X}")
    return green, red
