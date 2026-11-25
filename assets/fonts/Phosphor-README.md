# Phosphor Regular icon subset

Source: [Phosphor Icons web 2.1.2](https://github.com/phosphor-icons/web), MIT.
Full notice: [Phosphor-LICENSE.txt](Phosphor-LICENSE.txt). The packaging license
generator includes this notice automatically through its font-license glob.

`Phosphor-Regular-Subset.ttf` contains only the 26 icons listed with their upstream
Unicode code points in `phosphor-subset.txt` (5,988 bytes). The desktop loads it
as a separate icon-only family; UI and code fonts remain Inter and JetBrains Mono.

To regenerate, install Python fonttools 4.68.0 and run `python3 tool/subset-phosphor.py`.
The script checks the pinned upstream TTF's SHA-256 before subsetting and preserves
its timestamp. The 488,636-byte upstream font is needed only when regenerating;
it is not shipped or fetched at runtime. No browser, new Rust dependency, or SVG
rasterization is involved.
