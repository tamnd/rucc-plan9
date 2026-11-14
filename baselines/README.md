# Baselines

`r9 baseline` writes the files here (plan 12.1). It boots the G image of a pin three times for each suite and keeps the result of each test. The path of a file is `baselines/PIN/OBJTYPE/SUITE.json`. The suites are `boot`, `smoke` (the `smoke/suite.rc` script) and `test` (the mktest tests of the tree, run by `r9-test`).

The G image must be the G build of the pin, and not `g-base` itself. `g-base` builds only the compilers, the loaders, the assemblers and the libraries of the pin. The other programs in `g-base` are still the programs of the release. At 2859ae04e800, the awk of release 11952 fails 2 cases of `cmd/awk/test/T.builtin` that the awk of the pin passes, so the oracle would grade a correct R awk as a difference. Make the G build and the baselines with

```
r9 build --side G --row amd64 --keep --out work/g-amd64
r9 baseline --row amd64 --image work/g-amd64/root.qcow2 --pin 2859ae04e800
```

The baselines of 2859ae04e800 come from the G build of each row.

| key | meaning |
|---|---|
| `pin` | the pin of the tree |
| `objtype` | the objtype of the G image |
| `suite` | the name of the suite |
| `runs` | for each run: `passed` (the boot halted as the script ordered), `seconds`, `tests` (the number of results) and `sha256` (a hash of all of its results) |
| `results` | the result of each test that is the same in all runs: `ok`, `FAIL` or `value` with a hash of the output |
| `graded` | the number of tests in `results`. Only these tests grade R. |
| `graded_fail` | the number of graded tests that fail on G |
| `flaky` | the number of tests that do not give the same result in all runs |
| `flaky_results` | the result of each run for each flaky test |

For the `boot` suite, the `console` result is a hash of the console before the first prompt. Before the hash is made, `r9 baseline` changes each run of digits to `N`, and each word of hex digits with a digit in it to `N`. It removes the transfer rate that U-Boot writes and the lines that give the size of memory. It also removes the lines that identify the kernel: the line where the loader counts the kernel in with backspaces, the `.bootfs:` line with the build date, and the `fingerprint:` line of paqfs. Thus a kernel of a different build gives the same console value when it boots the same way. In `flaky_results`, `missing` tells that the test did not give a result in that run.
