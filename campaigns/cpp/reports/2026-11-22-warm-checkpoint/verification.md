# Checkpoint packaging verification

The portable analysis runs on the committed compact tables:

```text
PASS: 40 endpoint pairs, 30 bisect/mode pairs, 20 loss pairs; retained counters and object checks
PASS: all timing medians and bootstrap intervals reproduce the original analyses
PASS: profile completion-chain budgets close; CPU category percentages reproduce retained counts
CHECKPOINT_ANALYSIS_EXIT=0
```

Full gate, from the fresh `warm-checkpoint` worktree based on `9553a432`:

```sh
env -u TMPDIR -u KACHE_CACHE_DIR \
  KACHE_TEST_SCRATCH_DIR=/home/ubuntu/kache-test-scratch make check
```

The existing scratch override keeps explicit test runtime paths short. It does not
change production runtime selection or any acceptance rule. The gate completed with
exit zero. Its tail includes:

```text
test result: ok. 622 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.38s

running 5 tests
.....
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

running 1 test
.
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

MAKE_CHECK_EXIT=0
```

Local Markdown link checks passed. The report remains below 300,000 bytes.
`git diff --check` passed. The only paths changed are this report directory and
`campaigns/cpp/STATUS.md`. The acceptance plan and production source are unchanged.
No acceptance benchmark was rerun for packaging. No new macOS or musl proof is added.
