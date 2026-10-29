# MatRust alpha user guide

MatRust is a desktop numerical workspace written in Rust. It runs a subset of the MATLAB language. This guide is for engineers and students who already know some MATLAB.

## 1. What MatRust is and is not

MatRust is:

- A workspace with a command window, a script editor, figures, a variable list and a folder browser.
- An engine for numeric, logical, complex, integer, single, character, cell and struct data, with over 750 functions that each have a help page.
- A command-line program (`matrust`) that uses the same engine as the desktop.
- Built-in tools for filter design and for discrete-time block simulation.

MatRust is not:

- A full MATLAB replacement. It runs a subset of the language and does not run toolboxes.
- A Simulink replacement. The block tool is small and discrete-time only.
- Complete. Every function has stated limits, and a call outside those limits is rejected with an error rather than approximated.

What works today:

| Topic | Supported (examples, not the full list) |
| --- | --- |
| Language | `if/elseif/else`, `for`, `while`, `switch`, `try/catch`, `break`, `continue`, `return`, `global`, anonymous functions and function handles (`@sin`, `@(x) x.^2`), user functions with `nargin` and `nargout`, `eval`, `feval`. |
| Types | Double, single, `int8` to `int32` and `uint8` to `uint32`, `int64` and `uint64`, logical, complex, char, cell arrays, structs and struct arrays, `containers.Map`, `inputParser`. |
| Arrays | `zeros`, `ones`, `eye`, `linspace`, `reshape`, `repmat`, `cat`, `circshift`, `sort`, `sortrows`, `unique`, `ismember`, `union`, `intersect`, `find`, `any`, `all`, `accumarray`, `meshgrid`, `sub2ind`. Indexing, indexed assignment, growth and deletion (`A(1:10,:)=[]`). |
| Linear algebra | `A\b`, `det`, `inv`, `lu`, `qr`, `chol`, `eig`, `svd`, `expm`, `pinv`, `rank`, `null`, `kron`, `norm`, `cond`. |
| Math and statistics | Elementwise functions, `mod`, `rem`, `gamma`, `erf`, `besselj`, `primes`, `gcd`, `sum`, `mean`, `median`, `std`, `var`, `cumsum`, `cov`, `corrcoef`, `polyfit`, `polyval`, `roots`, `conv`, `interp1`, `spline`, `rand`, `randn`, `randi`. |
| Text | `sprintf`, `fprintf`, `num2str`, `mat2str`, `strsplit`, `strjoin`, `strrep`, `strtrim`, `regexp`, `regexprep`, `strcmp`, `upper`, `str2double`, `sscanf`, `dec2bin`. Character rows and cells only, no string arrays. |
| Signals | `fft`, `ifft`, `filter`, `filtfilt`, `conv`, `fir1`, `butter_sos`, `cheby1`, `freqz`, `grpdelay`, `impz`, window functions, `chirp`, `square`, `upfirdn`. |
| ODE, integration, optimisation | `ode45`, `ode23`, `ode15s`, `integral`, `integral2`, `quad`, `trapz`, `cumtrapz`, `fminsearch`, `fzero`, `fminbnd`, `optimset`. `fminsearch`, `fzero` and `fminbnd` take the function name as text (`'myfun'`), not a handle. |
| Data structures | `struct`, `fieldnames`, `isfield`, `rmfield`, `cellfun`, `arrayfun`, `cell2mat`, `struct2cell`, `containers.Map`. |
| Plots | `plot`, `scatter`, `bar`, `stairs`, `stem`, `semilogx`, `semilogy`, `loglog`, `subplot`, `tiledlayout`, `legend`, `grid`, `xlim`, `ylim`, `axis`. 2-D only. |
| Files | `.mat` save/load, `readmatrix`, `writematrix`, `fopen`, `fprintf`, `fgetl`, `fileread`, `savefig`, `openfig`, `saveas`, `exportgraphics`. |
| Dates and time | `datenum`, `datestr`, `datevec`, `weekday`, `clock`, `tic`, `toc`. |

There are over 750 functions in total. Type `help name` for a page, or see `assets/help/functions.md` in the source tree for the full list.

Limits to know about:

- Arrays are 2-D only. N-D arrays (`zeros(2,2,2)`, `cat(3,...)`) are rejected.
- Per-run limits: 1,000,000 execution steps; output is cut at 2,000 entries or 2 MiB, so end lines with a semicolon; at most 16 open figures and 64 series per figure.
- Function calls can nest up to 32 levels deep.
- Not available: toolboxes, Simulink, `classdef`, sparse matrices, string arrays (`"text"` cannot be stored in a variable; use single quotes), `parfor`, `table`, `datetime`, 3-D plots and images (`surf`, `plot3`, `image`), `histogram`, `text`. These report "not supported yet".
- Complex linear solves (`A\b` with complex `A`) are not supported.
- Some functions accept only some option forms; unsupported forms are rejected with an error that names the missing piece.
- `fir1` and `cheby1` follow the MATLAB call forms. `butter_sos(order,Wn[,type])` is MatRust's own function: it returns second-order sections directly. MATLAB `butter` is not available.

## 2. Install and start

### Windows desktop

1. Download the Windows zip from the [Releases page](https://github.com/leoom-labs/matrust/releases).
2. Unzip it to a folder you can write to, for example `C:\MatRust`.
3. Run `matrust-desktop.exe`.

The zip also holds `matrust.exe` (command line) and `matrust-pdf-worker.exe`. The desktop uses the PDF worker to preview PDF and EPS files. Keep it in the same folder as `matrust-desktop.exe`; the desktop looks for it only there.

The desktop remembers your session: open scripts (up to 32) and the active one, the Current Folder, recent files (up to 20), command history (up to 200 commands), panel layout, theme, the toolstrip tab, export settings, open figures, workspace variables and the open block model.

Linux and macOS: download the matching archive from the same page. The macOS build is unsigned: the first time, right-click `matrust-desktop` in Finder and choose Open.

### Command line

```
matrust                          interactive console (type exit or quit to leave)
matrust script.mr                run a script file
matrust --eval "disp(2+2)"       run one expression
matrust script.mr --threads 4    limit the worker threads
matrust script.mr --svg out.svg  save the current plot as SVG
matrust --help
```

- `matrust script.m` also works.
- `--threads N` takes a value from 1 to 64. The default is the number of CPU cores, capped at 8.
- A script file larger than 1 MB is rejected.
- Function files are looked up in the script's folder, or in the current folder for `--eval` and the console.
- A failing script prints its output so far, then `Error: ...`, and exits with a non-zero code.

## 3. The desktop

### Toolstrip

The toolstrip sits at the top. Three tabs hold grouped actions: Home, Plots and Apps. A menu bar with File, Workspace, Tools and Layout offers the same actions plus a few more. A Help menu sits at the top right, next to the Run script button. "Hide tools" collapses the strip; "Show tools" brings it back.

**Home**

| Group | Actions |
| --- | --- |
| Scripts | New script, Open script, Save script, Save script as, Reload script |
| Execution | Run script (Ctrl+Enter), Run selection (F9) |
| Workspace | Load workspace, Save workspace, Import CSV, Inspect variable |
| Desktop | Command Window main, Editor and figure, Editor only, Figure only, Reset layout |

**Plots**

| Group | Actions |
| --- | --- |
| Workspace data | Plot workspace data |
| Figures | New figure, Open figure, Save figure, Export figure |
| Edit | Figure properties, Undo figure edit, Redo figure edit, Reload figure |
| Layouts | 2 by 2 layout, Vertical layout |
| Images | Open plot image, Copy figure |

**Apps**

| Group | Actions |
| --- | --- |
| Signal processing | Filter Designer, Filtering example |
| Simulation | Block Simulation |
| Examples | Plotting example |

Filter Designer and Block Simulation open as tabs in the main work area.

### Command Window

Type an expression and press Enter. Results print below. Use a semicolon to hide output. `clc` clears the window, `whos` lists variables, `clear` removes them. Output is capped per run (section 1). A long calculation shows a Stop button in the toolstrip. Stopping is cooperative, so it takes effect at the next check point.

### Editor

The Editor holds script tabs.

- Ctrl+Enter runs the active script. F9 runs the selected text.
- Ctrl+S saves, Ctrl+Shift+S saves as. "Save all scripts" is Ctrl+Alt+S.
- Ctrl+Tab and Ctrl+Shift+Tab move between script tabs.
- Opening a script never runs it. Reopening a file that is already open selects its tab and keeps your unsaved edits.
- The Editor can be shown beside the figure or alone (Home > Desktop). The "Undock Editor" button in the Editor header moves it to a separate window; "Dock back" returns it.

### Figure

Plots from scripts and the Command Window appear in figure tabs. The Plots tab holds the figure actions. Ctrl+click a point on a plot to inspect the nearest original sample. The "Sample inspection" window shows the axes, series, sample index and X/Y values. Figure edits can be undone and redone.

### Workspace

The Workspace panel lists variables with name, size and class. Double-click a variable, or press Enter, to inspect it. In the inspector you can edit single values of real, logical, complex and character arrays, including values inside cells. Edits do not change the array size. The Workspace menu offers Rename variable, Delete selected variables, Clear workspace, Load workspace, Save workspace and Import CSV. F2 renames the selected variable. Layout > Workspace position docks the panel at the Right or Below Files.

### Current Folder

The left rail switches between Files, History and Examples.

- Files shows the Current Folder. Scripts, `.mat`, CSV, figure and image files open from here.
- The Current Folder bar has Back, Forward and Up buttons, an address field and Browse. The Files list has a filename filter. F5 refreshes. Alt+Left, Alt+Right and Alt+Up navigate. Ctrl+L moves to the address field.
- Relative paths in scripts and the commands `load`, `save`, `readmatrix` and so on resolve from the Current Folder.
- A folder with more than 100,000 matching entries shows a limit notice; narrow the filter.

### Layout and theme

Layout > Appearance selects Light, Dark or "Use system setting". Reset layout restores the default panel arrangement and keeps your documents and data. Ctrl+F6 and Ctrl+Shift+F6 move between the visible focus areas (Files, Editor, figure, Command Window and others). Ctrl+/ or Help > Keyboard shortcuts shows the navigation shortcut list.

### Status bar, empty states and About

- The status bar at the bottom shows what the app is doing ("Ready", "Running... Press Stop to cancel", "Completed in N ms") and the number of variables.
- With no script open, the Editor shows New script and Open script links and your recent files. With no figure, the figure panel shows a hint.
- Help > About MatRust shows the version and third-party notices.

## 4. Scripts and functions

### File types

- `.mr` is MatRust's own script and function file. New scripts are saved as `.mr` by default.
- `.m` files still open, run and save. Saving a file that was opened as `.m` keeps the `.m` name.
- If the folder holds both `name.mr` and `name.m`, `name.mr` wins.

### Running

- In the Editor, Run script (Ctrl+Enter) runs the whole text as it is in the editor, including unsaved changes. Run selection (F9) runs only the selected lines.
- From the command line, `matrust myscript.mr`.
- Typing a script's name in the Command Window runs it.
- `mfilename`, `fileparts` and `fullfile` work for path handling. Running does not change the Current Folder.

### Functions

```
function [s, p] = sumprod(a, b)
    s = a + b;
    p = a * b;
end
```

- A function file has the same name as the function: `sumprod.mr` in the Current Folder.
- Local helper functions can follow the main function in the same file.
- `nargin`, `nargout` and `return` work. Each call has a private workspace.
- Nesting is limited to 32 call levels.
- Each function file must be under 1 MB.

### Structs, cells, handles, try/catch and switch

```
s.name = 'Ada';
s.scores = [90 85 77];
disp(mean(s.scores))
t = struct('a', {1, 2, 3});          % 1x3 struct array

c = {1, 'two', [3 4 5]};
c{end+1} = {6};

sq = @(x) x.^2;
disp(sq(4))
disp(cellfun(@(v) numel(v), c))
disp(arrayfun(@(k) k*2, 1:4))

try
    error('demo:bad', 'Value %d is bad', 7);
catch err
    disp(err.identifier)
    disp(err.message)
end

for k = 1:3
    switch k
        case 1
            disp('one')
        case {2, 3}
            disp('two or three')
        otherwise
            disp('other')
    end
end
```

`switch` works on numbers and on text. Structs cannot be saved to `.mat` files yet (section 6).

### Errors

An error stops the run and prints a message. Output produced before the error is kept. The Command Window shows the error in red. In the Editor, the line number of the failing line turns red, and a "Go to error" button jumps to it.

Common messages name the unsupported piece, for example "Supported saveas formats: ...". They are intended to tell you what to change.

### Benchmarking

- `matrust bench` times the built-in suite. `--script myscript.m` or `--op "y = f(x);"` times your own code.
- `--share FILE.json` prints MatRust's numbers for a public GitHub issue that you submit yourself.
- Method, every option and how to read the table: [BENCHMARKS.md](../../BENCHMARKS.md). What sharing sends: [privacy](../privacy.md).

## 5. Figures

### Creating plots

```
x = 0:0.01:1;
figure(1);
subplot(2,1,1); plot(x, sin(2*pi*5*x), 'LineWidth', 2);
title('Sine'); xlabel('t'); grid on
subplot(2,1,2); stairs(x, round(x*4)/4);
```

- `plot`, `scatter`, `bar`, `stairs`, `stem`, `semilogx`, `semilogy`, `loglog`, `hold on/off`, `legend`, `grid`, `xlim`, `ylim`, `axis`, `title`, `xlabel`, `ylabel`.
- `subplot(rows,cols,index)` and `subplot('Position',[l b w h])`.
- `tiledlayout(rows,cols)`, `tiledlayout('flow')`, `nexttile`, `sgtitle`.
- Complex input to `plot(Z)` draws real against imaginary.
- Colours of new series follow the light or dark theme unless you set them.

### Saving figures

| Command | Result |
| --- | --- |
| `savefig('name.mrfig')` | MatRust native editable figure. |
| `savefig('name.fig')` | MATLAB `.fig` (restricted to what MatRust can write: 2-D axes). |
| `openfig('name.mrfig')` | Opens the figure. `.mfig` (the older MatRust extension) and `.fig` also open. |

`savefig('name')` with no extension writes `name.fig`, as in MATLAB. In the desktop, Save figure suggests `.mrfig` for a new figure. It offers `.fig` only when the figure fits what MatRust can write. Saving a figure that already has a file keeps that file's format.

### Exporting images

```
saveas(gcf, 'plot.png')
saveas(gcf, 'plot', 'svg')
exportgraphics(gcf, 'plot.png', 'Resolution', 300)
```

`saveas` formats: `fig`, `png`, `jpg`/`jpeg`, `svg`, `pdf`, `bmp`, `tif`/`tiff`/`tiffn`, and `epsc` (colour EPS; only when given as an explicit format argument). Monochrome and Level-2 EPS, EMF, and MATLAB M/MFIG pairs are rejected.

`exportgraphics` writes a tight crop of the figure. File types: PNG, JPEG, TIFF, PDF and SVG. Options: 'Resolution', 'ContentType' and 'BackgroundColor' (white, current or 'none' only). 'Append', custom background colours, 'Colorspace', size, padding and units are not available.

Raster, PDF and EPS export need the `plot-export` build feature, which the desktop build includes. SVG works in every build. In the desktop, Export figure offers PNG, JPEG, SVG, PDF, colour EPS, TIFF, BMP, plot data CSV and a MATLAB plot script. You pick a size (64 to 8192 pixels per side, up to 32 million pixels) and can export all open figures to a folder. Copy figure puts an image on the clipboard.

## 6. Data files

### MAT files

```
save('data.mat', 'A', 'mask', '-v7.3')
save data.mat A mask
load('data.mat', 'A')
load data.mat
```

- `-v6`, `-v7` (the default, compressed), `-v7 -nocompression` and `-v7.3` (HDF5) are supported. `-v7.3 -nocompression` is rejected.
- Supported contents: real, logical, complex, integer and single arrays, character arrays and nested cell arrays.
- Without a filename, `save` and `load` use `matlab.mat`. A name with no extension gets `.mat`.
- Not supported yet: structs, `S = load(...)` (assigning the result), ASCII, `-append`, wildcard and regexp selection, string variables.
- `load file.dat -mat` reads MAT content from a file with another extension.
- In the desktop, Workspace > Load workspace and Save workspace do the same from menus. Save workspace offers "MATLAB v7 (Level-5)" and "MATLAB v7.3 (HDF5)".

### CSV and text matrices

```
A = readmatrix('data.csv');
writematrix(A, 'out.csv');
```

- Text must be strictly numeric with comma, semicolon or tab separators. A header row, an empty cell, a space separator or rows of different length stop the read with an error. `NaN` and `Inf` are read as numbers.
- `writematrix` writes real and logical arrays. It does not write character, cell or complex arrays.
- `writematrix` replaces an existing file without asking. If the write fails, the old file is kept. Files are limited to 512 MiB.

In the desktop, Import CSV (Home > Workspace, or the Workspace menu) opens a file picker, then asks for a variable name. Reading runs in the background and can be stopped.

### Other file types the desktop opens

`.txt`, `.mrsim` (block models), `.slx` (supported simple discrete graphs, imported as an unsaved native model), and image previews: PNG, JPEG, BMP, TIFF, static SVG and PDF. EPS previews need Ghostscript, set up in the app.

## 7. Filter Designer and Block Simulation

Open both from Apps or from the Tools menu. Each opens as a tab in the main work area.

### Filter Designer

Designs a digital filter and shows its response.

Settings:

- Type: "FIR · windowed" or "IIR · SOS" (second-order sections).
- IIR design family: Butterworth or Chebyshev I (with passband ripple in dB).
- Response: low, high, bandpass, stop.
- FIR window: Hamming, Hann, Blackman, Bartlett, Triangular, Rectangular, Blackman-Harris, Nuttall, Flat top. "Scale passband gain" is an option, on by default.
- Order: 1 to 4096 for FIR; 1 to 20 for IIR. Band responses of IIR double the order, with at most 20 sections.
- Sample rate, cutoff (or lower edge) and upper edge, all in Hz. The upper edge shows only for bandpass and stop.
- Workspace input variable (default `x`).
- Analysis: magnitude (dB), phase, group delay, impulse response, step response; 1 to 65,536 points.

Buttons:

- **Design + response** plots the chosen analysis.
- **Apply to y** filters the named workspace variable. It creates `b` (FIR) or `sos` (IIR) and the filtered result in `y` (replacing any existing `y`), then plots `y`. The input must be a real numeric variable.
- **Save design script…** writes a `.mr` (or `.m`) script with the design and analysis calls, so you can rerun it without the tool.
- **Add to block model** adds the filter as a block in Block Simulation.

Walk-through:

1. In the Command Window: `t = 0:1/1000:1; x = sin(2*pi*5*t) + 0.5*sin(2*pi*200*t);`
2. Apps > Filter Designer.
3. Choose IIR, Butterworth, low, order 4, sample rate 1000, cutoff 30.
4. Press Design + response, then Apply to y. The input variable is `x`.
5. The plot of `y` shows the 5 Hz sine with the 200 Hz part removed.

Apps > Filtering example opens a sample script in a new Editor tab; it does not run it. The script filters a two-channel signal in two chunks, carrying the filter state between them, and checks the result against one pass. Apps > Plotting example opens a tiled-layout plotting script the same way.

### Block Simulation

A small discrete-time block editor. One fixed sample rate, scalar signals.

- Blocks: Constant, Sine, Step, Gain, Sum, Unit Delay, Discrete Filter, SOS Filter, Scope, Abs, Bias, Saturation, Product, Unary Minus, Switch, Discrete-Time Integrator (forward Euler).
- Feedback loops must pass through a Unit Delay or an Integrator.
- Up to 512 blocks, up to 1,000,000 samples.
- Files: New model, Open, Save model, Save as. Native format is `.mrsim`. SLX files import as an unsaved model; Save as `.mrsim` to keep it.

Walk-through:

1. Apps > Block Simulation.
2. Pick Sine from the block list and press Add block. Add Gain, then Scope.
3. In the Parameters tab, connect each block input from its drop-down (for example, Gain "Input 1": Sine).
4. Set Sample time (s) and Samples.
5. Press Run model. The status line must read "Model valid · ready to simulate".
6. The first Scope plots in a figure. Workspace variables `sim_t`, `sim_1`, `sim_2`, ... hold time and the Scope traces, in Scope order.

Add to block model in Filter Designer puts the designed filter into the open model (a Discrete Filter block for FIR, an SOS Filter block for IIR). Connect its input and a Scope.

## 8. Not available yet

These are not in the engine. A call reports an error that says the feature is not supported:

- Toolboxes and Simulink models beyond the small block tool in section 7.
- `classdef`, `parfor`, sparse matrices, N-D arrays, string arrays (`"text"` variables).
- `table`, `datetime`, `categorical`.
- 3-D plots, images, `histogram`, `text`, `colorbar`.
- Saving structs to `.mat` files, `S = load(...)`, `save -append`.
- Function handles as the first argument of `fminsearch`, `fzero` and `fminbnd`; pass the function name as text.
- Complex linear solves with `\`.

## 9. Getting help and reporting problems

- `help` in the Command Window prints a short summary of supported syntax.
- `help name` (or `doc name`) prints a help page. Every supported function has a page, and `assets/help/functions.md` lists them all. For your own function files it prints the leading `%` comment lines.
- `matrust --help` prints the same summary, plus the command-line usage.
- Ctrl+/ lists navigation shortcuts.
- Error messages state what is unsupported. Copy the message when reporting.

Report issues at https://github.com/leoom-labs/matrust/issues (see `CONTRIBUTING.md` for what to include).

When you report a problem, include: the MatRust version (Help > About MatRust), Windows version, the script or steps, and the full error text.
