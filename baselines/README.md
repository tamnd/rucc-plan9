# Baselines

`r9 baseline` writes the files here (plan 12.1). It boots the G image of a pin three times for each suite and keeps the result of each test. The path of a file is `baselines/PIN/OBJTYPE/SUITE.json`. The suites are `boot`, `smoke` (the `smoke/suite.rc` script) and `test` (the mktest tests of the tree, run by `r9-test`).

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

For the `boot` suite, the `console` result is a hash of the console before the first prompt. Before the hash is made, `r9 baseline` changes each run of digits to `N`, removes the transfer rate that U-Boot writes, and removes the lines that give the size of memory. In `flaky_results`, `missing` tells that the test did not give a result in that run.
