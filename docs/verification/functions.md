# The function oracle

**What only it catches.** A function, or a jump table target, that should have
been found and was not. Everything downstream works on what discovery found, so a
function discovery misses is not lifted and not drawn from, and nothing later can
notice it was ever there.

**Input.** A shipped title, plus jump tables and helper addresses worked out by
hand elsewhere.

**Pass condition.** The recovered jump tables agree with the hand worked
reference, target for target, and the helper addresses are the ones recorded.
Disagreements are counted in both directions: a table this recovers and the
reference does not is as much a defect as the other way round.

**Command.**

```sh
XENOLITH_ANALYSIS_XEX=.../ac6_extract/default.xex \
XENOLITH_ANALYSIS_HELPERS=... \
XENOLITH_ANALYSIS_SWITCHES=... \
XENOLITH_ANALYSIS_TABLE=... \
  cargo test --release -p xenolith-analysis -- --include-ignored
```

**What it is checking.** Discovery claims code by several routes: following
calls, scanning for a prologue, reading a jump table, and claiming an address the
image points at. Each route has its own way of being wrong, and the failure is
silent: the words stay unclaimed, nothing branches to them, and the only symptom
is a program that stops at an address it never lifted.

**Limit.** The reference tables were worked out by hand, so they cover the tables
someone chose to work out. A table neither side finds is not a disagreement. The
shortfall is reported rather than hidden: the analysis says how many executable
words it claimed, and what remains has been measured to be data, addresses, or
leaf functions with no prologue to find them by.
