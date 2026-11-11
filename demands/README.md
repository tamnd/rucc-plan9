# Demands

The files here are written by `r9 demands` (plan 05.6). Each file ranks the first errors that rucc gives on the units of one G build, by the number of units that each error stops. The directory name is the pin of the tree.

| file | build | rucc |
|---|---|---|
| `OBJTYPE.toml` | the G build of OBJTYPE | `-fsyntax-only` on the host target of the same architecture, with no Plan 9 options |
| `OBJTYPE-kencc.toml` | the G build of OBJTYPE | the kencc driver with the Plan 9 target, as `r9 ccd` runs it |

The first file shows what the tree demands from a compiler that knows only GNU C. The second file shows what is left for rucc with its Plan 9 options. Both files at 2859ae04e800 come from rucc 0.30.15.

For the kencc files, the root is the tree at the pin, with the include directories of every objtype and the generated C files of the arm64 G build put in place. The errors that are left are data files that a build makes and that the G build did not keep, such as `runebreakdata` and `rgbv.h`.
