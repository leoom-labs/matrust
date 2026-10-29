# MatRust function help

One page per function. A page runs from its "## name" heading to the next
"## " heading. The first line after the heading is the summary. Lines are
printed as written.

## abs
Absolute value or complex magnitude.
Syntax: abs(X)
Returns the absolute value of each element. For complex input it returns the magnitude as a real array.
Example: abs([-1 2 -3])   % [1 2 3]
Differences from MATLAB: Real double or complex 2-D arrays only; logical input is rejected.

## all
True if all elements are nonzero.
Syntax: all(A)
For a vector it returns one logical value. For a matrix it returns a row with one value per column.
Example: all([1 1 0])   % false
Differences from MATLAB: No dim argument; real or logical 2-D arrays.

## and
Element-wise logical AND.
Syntax: and(A, B)
Same as A & B. Inputs are converted to logical.
Example: and([1 0], [1 1])   % [true false]
Differences from MATLAB: Exactly two arguments; more than two are not accepted.

## angle
Phase angle in radians.
Syntax: angle(Z)
Returns the angle of each element of a real or complex array. Real input gives 0 or pi.
Example: angle(-1)   % 3.1416
Differences from MATLAB: none known.

## annotation
Add a text box to a figure.
Syntax: t = annotation(fig, 'textbox', [x y w h], 'String', text, 'Interpreter', 'none', 'EdgeColor', 'none')
The rectangle is in normalized figure units. Interpreter must be 'none' and EdgeColor must be 'none'. Properties such as HorizontalAlignment and FontWeight are also accepted.
Example: f = figure; t = annotation(f, 'textbox', [0 0.9 1 0.1], 'String', 'Note', 'Interpreter', 'none', 'EdgeColor', 'none');   % text box at the top of the figure
Differences from MATLAB: Not checked against MATLAB yet. Only the 'textbox' type; the TeX interpreter and the default outline are not supported yet.

## any
True if any element is nonzero.
Syntax: any(A)
For a vector it returns one logical value. For a matrix it returns a row with one value per column.
Example: any([0 0 2])   % true
Differences from MATLAB: No dim argument; real or logical 2-D arrays.

## atan2
Four-quadrant inverse tangent in radians.
Syntax: atan2(Y, X)
Returns the angle of the point (X,Y), including the signs of both coordinates. Inputs must have the same size, or one must be scalar. Real numeric and logical inputs are accepted; complex input is rejected.
Example: atan2(1,0)   % 1.570796
Differences from MATLAB: none known.

## axes
Create axes or make axes current.
Syntax: axes
Syntax: h = axes()
Syntax: h = axes('Position', [left bottom width height])
Syntax: axes(h)
With no arguments it creates axes at the default position [0.13 0.11 0.775 0.815] in the current figure, or in a new figure if there is none. Name-value pairs: Parent (a figure handle), Position (normalized), Units ('normalized'), NextPlot ('add' or 'replace'). axes(h) makes the axes h current.
Example: figure; h = axes('Position', [0.1 0.1 0.8 0.8]); get(h, 'Position')   % [0.1 0.1 0.8 0.8]
Differences from MATLAB: Not checked against MATLAB yet. h = axes without parentheses is not supported yet; write h = axes(). Axes may not overlap, and other properties are rejected.

## axis
Set axis limits of the current plot.
Syntax: axis([xmin xmax ymin ymax])
Syntax: axis tight
Syntax: axis auto
Sets both axis ranges, or returns them to automatic limits. Commands such as axis tight also work in command form.
Example: axis([0 10 -1 1])   % x from 0 to 10, y from -1 to 1
Differences from MATLAB: Only numeric limits, auto and tight; other options such as equal or off are rejected.

## bar
Bar plot.
Syntax: bar(y)
Syntax: bar(x, y)
Draws one bar per value. A single color letter or the properties BarWidth, FaceColor, EdgeColor and LineStyle can follow the data.
Example: bar([3 5 2])   % three bars
Differences from MATLAB: Only a single color letter is accepted, not full line specs. Up to 16 figures and 64 series.

## bartlett
Bartlett window.
Syntax: bartlett(n)
Returns an n-by-1 column vector. The length is rounded to an integer.
Example: bartlett(5)'   % [0 0.5 1 0.5 0]
Differences from MATLAB: No sampling option; double output only.

## blackman
Blackman window.
Syntax: blackman(n)
Syntax: blackman(n, 'periodic')
Returns an n-by-1 column vector. The default is symmetric; 'symmetric' is also accepted.
Example: w = blackman(64);   % 64-by-1 column
Differences from MATLAB: Double output only.

## blackmanharris
Minimum 4-term Blackman-Harris window.
Syntax: blackmanharris(n)
Syntax: blackmanharris(n, 'periodic')
Returns an n-by-1 column vector. The default is symmetric.
Example: w = blackmanharris(64);   % 64-by-1 column
Differences from MATLAB: Double output only.

## blanks
Text of n spaces.
Syntax: s = blanks(n)
Returns a 1-by-n character row of spaces. n is an integer from 0 to 65536.
Example: size(blanks(3))   % [1 3]
Differences from MATLAB: Not checked against MATLAB yet.

## break
Leave the innermost for or while loop.
Syntax: break
Example: for k = 1:5, if k == 3, break, end, end, k   % k = 3
Differences from MATLAB: Not checked against MATLAB yet.

## butter_sos
Butterworth filter as second-order sections.
Syntax: sos = butter_sos(order, Wn)
Syntax: sos = butter_sos(order, Wn, type)
Wn is a normalized cutoff (Nyquist = 1). One cutoff gives 'low' (default) or 'high'; two increasing edges give 'bandpass' (default) or 'stop'. A band design has order 2*order. Order is 1 to 20.
Example: sos = butter_sos(4, 0.2)   % 2-by-6 SOS matrix
Differences from MATLAB: MATLAB has no butter_sos. It returns the SOS matrix directly; there is no butter function.

## cast
Convert to another class.
Syntax: cast(X, CLASSNAME)
CLASSNAME is 'double', 'single', 'logical', 'char' or an integer class name. Integer targets round half away from zero and saturate.
Example: x = cast(300, 'uint8')
Differences from MATLAB: the 'like' form is not supported yet.

## catch
Handle an error from a try block.
Syntax: try; statements; catch; handler; end
Syntax: try; statements; catch err; handler; end
Runs the handler only if the try block raises a catchable error. The optional err variable is a scalar struct with message, identifier and stack fields. Its stack is an empty 0-by-1 struct with file, name and line fields. An error in the handler propagates to an enclosing try block.
Example: try; error('plain msg'); catch err; disp(class(err)); end   % struct
Differences from MATLAB: The caught value is a struct, not an MException. Cancellation and execution limits cannot be caught.

## ceil
Round toward positive infinity.
Syntax: ceil(X)
Rounds each element up to an integer.
Example: ceil(2.1)   % 3
Differences from MATLAB: Real and complex double 2-D arrays. Complex results with zero imaginary parts are real.

## cell
Create a cell array.
Syntax: C = cell(n)
Syntax: C = cell(rows, cols)
Syntax: C = cell([rows cols])
Creates cells that each hold an empty array []. Dimensions are integers from 0 to 65536. Fill cells with C{k} = value and read them with C{k}.
Example: C = cell(1, 3); C{2} = 'x'; size(C)   % [1 3]
Differences from MATLAB: Not checked against MATLAB yet. Two-dimensional output only.

## char
Convert character codes to text, or stack text rows.
Syntax: s = char(codes)
Syntax: s = char(C)
codes is an array of integers from 0 to 65535 (UTF-16 units); the result keeps its shape. Character input is returned unchanged. A cell array of character rows becomes a character matrix with one row per cell, in column-major order, shorter rows padded with spaces.
Example: char([72 105])   % 'Hi'
Example: char({'a', 'bcd'})   % rows 'a  ' and 'bcd'
Differences from MATLAB: Not checked against MATLAB yet. One input only (use strvcat for several); non-integer and out-of-range codes are rejected.

## cheby1
Chebyshev type I digital filter design.
Syntax: b = cheby1(n, Rp, Wp)
Syntax: [b, a] = cheby1(n, Rp, Wp, type)
Syntax: [z, p, k] = cheby1(n, Rp, Wp)
Rp is the passband ripple in dB. Wp is one or two normalized edges. type is 'low', 'high', 'bandpass' or 'stop'.
Example: [b, a] = cheby1(4, 1, 0.3)   % 5 coefficients in each of b and a
Differences from MATLAB: Digital filters only, order 1 to 20. No analog, state-space or 'ctf' forms.

## cheby1_sos
Chebyshev type I filter as second-order sections.
Syntax: sos = cheby1_sos(n, Rp, Wp)
Syntax: sos = cheby1_sos(n, Rp, Wp, type)
Same design as cheby1, returned as an SOS matrix with six columns. Prefer it for high orders.
Example: sos = cheby1_sos(4, 1, 0.3)   % 2-by-6 SOS matrix
Differences from MATLAB: MATLAB has no cheby1_sos. One output only; digital filters only.

## chirp
Swept-frequency cosine.
Syntax: x = chirp(t)
Syntax: x = chirp(t, f0, t1, f1, method, phase, shape)
Defaults are f0 = 0, t1 = 1, f1 = 100. method is 'linear', 'quadratic' or 'logarithmic'; shape ('convex' or 'concave') applies to quadratic only. [] selects a default.
Example: x = chirp(0:0.001:1, 0, 1, 100);   % linear sweep from 0 to 100 Hz
Differences from MATLAB: Real output only; the 'complex' option is rejected.

## circshift
Shift array elements with wraparound.
Syntax: circshift(A, K)
Syntax: circshift(A, [rowShift colShift])
Syntax: circshift(A, K, dim)
Positive shifts move elements toward larger indices. A scalar K without dim shifts rows, even for a row vector. Use dim 2 to shift columns. Shifts must be finite integers. Numeric, complex, logical, character, cell and struct arrays are supported and keep their class. Extra vector shift entries must be zero; an explicit dim above 2 leaves A unchanged.
Example: circshift([1 2 3],1,2)   % [3 1 2]
Differences from MATLAB: MatRust prints MATLAB's warning for a scalar shift of a row vector only for complex input. Arrays are two-dimensional.

## class
Name of the type of a value.
Syntax: name = class(A)
Returns 'double', 'logical', 'char', 'cell' or 'graphics handle'.
Example: class('a')   % 'char'
Differences from MATLAB: Not checked against MATLAB yet. Complex values report 'double'. A graphics handle reports 'graphics handle' instead of a class such as 'matlab.graphics.axis.Axes'.

## clc
Clear the console.
Syntax: clc
Removes the text shown in the console. Variables are kept.
Example: clc   % console is empty
Differences from MATLAB: none known.

## clear
Remove variables from the workspace.
Syntax: clear
Syntax: clear A B
Syntax: clear('A', 'B')
Syntax: clear global [names]
With no names it removes all variables. Names must be literal text. A name that is not a variable unloads the function file of that name and resets its persistent variables. clear global removes global variables.
Example: clear A B   % removes A and B
Differences from MATLAB: clear all, functions, classes, import, java and mex are rejected.

## clf
Clear the current figure.
Syntax: clf
Removes all plots from the current figure. It takes no arguments.
Example: clf   % current figure is empty
Differences from MATLAB: No figure handle argument and no 'reset' option.

## close
Close a figure.
Syntax: close
Syntax: close(n)
Syntax: close all
With no argument it closes the current figure. n is a figure number from 1 to 1000000.
Example: close all   % all figures are closed
Differences from MATLAB: Takes a figure number or 'all' only.

## complex
Build a complex array.
Syntax: complex(re, im)
Syntax: complex(z)
Combines real and imaginary parts. The result stays complex even if the imaginary part is zero.
Example: complex(1, 2)   % 1 + 2i
Differences from MATLAB: Real, logical and complex 2-D arrays only.

## conj
Complex conjugate.
Syntax: conj(Z)
Negates the imaginary part. Real and logical input is returned unchanged.
Example: conj(1+2i)   % 1 - 2i
Differences from MATLAB: none known.

## continue
Skip to the next pass of the innermost for or while loop.
Syntax: continue
Example: for k = 1:3, if k < 3, continue, end, disp(k), end   % prints: 3
Differences from MATLAB: Not checked against MATLAB yet.

## cos
Cosine of an angle in radians.
Syntax: cos(X)
Applies cosine to each element; complex input is supported.
Example: cos(0)   % 1
Differences from MATLAB: double 2-D arrays only.

## cross
Cross product of three-element vectors.
Syntax: cross(A, B)
Syntax: cross(A, B, dim)
Without dim, three-element vectors may have different orientations; the result is a row if either input is a row. Otherwise inputs must have the same size, and the first dimension of length 3 is used. An explicit dim must have length 3. Real, complex and logical inputs are accepted.
Example: cross([1 0 0],[0;1;0])   % [0 0 1]
Differences from MATLAB: Only 2-D arrays are supported.

## ctranspose
Complex conjugate transpose.
Syntax: ctranspose(A)
Same as A'. Real and logical arrays are simply transposed.
Example: ctranspose([1+2i 3])   % [1-2i; 3]
Differences from MATLAB: none known.

## cumprod
Cumulative product.
Syntax: cumprod(A)
Syntax: cumprod(A, dim)
Works along the first non-singleton dimension unless dim is given.
Example: cumprod([1 2 3 4])   % [1 2 6 24]
Differences from MATLAB: Real and complex 2-D arrays; dim 3 or higher returns A unchanged. A complex result stays complex.

## cumsum
Cumulative sum.
Syntax: cumsum(A)
Syntax: cumsum(A, dim)
Works along the first non-singleton dimension unless dim is given.
Example: cumsum([1 2 3])   % [1 3 6]
Differences from MATLAB: Real and complex 2-D arrays; dim 3 or higher returns A unchanged. A complex result stays complex even when its imaginary parts are zero.

## cumtrapz
Cumulative trapezoidal integral.
Syntax: cumtrapz(Y)
Syntax: cumtrapz(X, Y)
Syntax: cumtrapz(Y, dim) or cumtrapz(X, Y, dim)
Returns the running integral of Y. X is a vector of sample points; without X the spacing is 1.
Example: cumtrapz([1 2 3])   % [0 1.5 4]
Differences from MATLAB: 2-D data; scalar spacing or a vector X. Complex Y works with a real vector X, where a scalar X is an error as in MATLAB; complex X is not supported.

## deblank
Remove trailing spaces from text.
Syntax: s = deblank(s)
Syntax: c = deblank(c)
Removes trailing white space and null characters. A character matrix loses the trailing columns that are blank in every row. A cell array of text gives a cell array.
Example: deblank('ab   ')   % 'ab'
Differences from MATLAB: Most cases are not checked against MATLAB yet.

## det
Determinant.
Syntax: det(A)
Returns the determinant of a square matrix, computed by LU factorization.
Example: det([1 2; 3 4])   % -2 (up to rounding)
Differences from MATLAB: real input must be double with finite values. Complex double input works as in MATLAB (LU pivoting by |re|+|im|; a zero imaginary part gives a real result; det(complex([])) is 1).

## diag
Diagonal matrix or diagonal of a matrix.
Syntax: diag(v)
Syntax: diag(v, k)
A vector gives a square matrix with v on diagonal k. A matrix gives its diagonal k as a column. k = 0 is the main diagonal.
Example: diag([1 2])   % [1 0; 0 2]
Differences from MATLAB: Real 2-D arrays only; only the diag(v,k) and diag(A,k) forms.

## diff
Differences between neighboring elements.
Syntax: diff(A)
Syntax: diff(A, n)
Syntax: diff(A, n, dim)
n is the order (default 1; [] means 1). The default dimension follows MATLAB R2025a and later.
Example: diff([1 4 9])   % [3 5]
Differences from MATLAB: Real and complex 2-D arrays; dim must be 1 or 2. A complex difference with zero imaginary parts is real.

## disp
Display a value.
Syntax: disp(X)
Prints X to the console without showing its name.
Example: disp('hello')   % prints hello
Differences from MATLAB: Exactly one argument. Output is bounded to 2000 entries or 2 MiB per run.

## doc
Show help for a function. Same as help.
Syntax: doc
Syntax: doc name
doc alone prints the overview of what MatRust supports. doc name prints the same page as help name.
Example: doc sin   % prints the page of sin
Differences from MATLAB: doc does not open a browser.

## dot
Dot product, conjugating the first input.
Syntax: dot(A, B)
Syntax: dot(A, B, dim)
Vectors of equal length give a scalar, even when their orientations differ. Otherwise inputs must have the same size and are summed along the first nonsingleton dimension, or the specified dim. A dim above 2 returns conj(A).*B. Real, complex and logical inputs are accepted.
Example: dot([1 2 3],[4;5;6])   % 32
Differences from MATLAB: Only 2-D arrays are supported. A complex result with zero imaginary part is real.

## double
Convert to double.
Syntax: double(X)
Converts logical values to 0 and 1 and characters to their UTF-16 codes, keeping the shape. Double and complex input is returned unchanged.
Example: double(true)   % 1
Example: double('Hi')   % [72 105]
Differences from MATLAB: Only logical, numeric and character arrays; other classes, including double-quoted strings, are not converted.

## downsample
Keep every n-th sample.
Syntax: downsample(x, n)
Syntax: downsample(x, n, phase)
phase is an integer from 0 to n-1 and selects the first kept sample.
Example: downsample([1 2 3 4 5], 2)   % [1 3 5]
Differences from MATLAB: Real numeric 2-D arrays only.

## eig
Eigenvalues and eigenvectors.
Syntax: e = eig(A)
Syntax: [V, D] = eig(A)
Syntax: [V, D, W] = eig(A)
Syntax: e = eig(A, B)
Syntax: [V, D] = eig(A, B, algorithm)
Syntax: [...] = eig(..., 'vector'), eig(..., 'matrix'), eig(A, 'nobalance')
Real symmetric and complex Hermitian A use the symmetric solver (ascending eigenvalues); other matrices use balancing, Hessenberg reduction and the QR algorithm, like LAPACK dgeev/zgeev. W holds left eigenvectors (W'*A = D*W'). eig(A, B) solves A*V = B*V*D; algorithm is 'chol' (default for symmetric A with symmetric positive definite B) or 'qz'. Single input gives single-rounded results.
Example: eig([2 1; 1 2])   % [1; 3]
Example: [V, D] = eig([2 0; 0 3], [1 0; 0 2]); x = diag(D)'   % [1.5 2]
Differences from MATLAB: Some results differ in the last printed digit. Single input is computed in double and rounded, so the last single digit can differ. Real results retain single storage; complex results use double storage with single rounding. Sparse input is not supported.
## else
Run statements when no earlier condition of an if was true.
Syntax: if condition, ..., else, ..., end
Used inside if, after the last elseif.
Example: x = 1; if x > 2, disp('big'), else, disp('small'), end   % prints: small
Differences from MATLAB: Not checked against MATLAB yet.

## elseif
Test another condition when the if condition is false.
Syntax: if c1, ..., elseif c2, ..., end
Syntax: if c1, ..., elseif c2, ..., else, ..., end
Used inside if. The first true condition runs its block and the rest are skipped. Write elseif as one word.
Example: x = 3; if x < 2, disp('a'), elseif x < 4, disp('b'), else, disp('c'), end   % prints: b
Differences from MATLAB: Not checked against MATLAB yet.

## end
Close a block, or the last index.
Syntax: if ... end
Syntax: for ... end
Syntax: while ... end
Syntax: function ... end
Syntax: A(end)
Syntax: A(end+1) = value
Closes if, for, while and function blocks. Inside an index, end is the last index of that dimension. Assigning to end+1 grows the array.
Example: x = [10 20 30]; x(end+1) = 40   % x = [10 20 30 40]
Differences from MATLAB: Not checked against MATLAB yet.

## eps
Spacing of numbers near 1.
Syntax: eps
Syntax: eps('single')
Syntax: eps(single(X))
Without an argument the constant is 2^-52, about 2.2204e-16. Single precision at 1 has spacing 2^-23.
Example: eps < 1e-15   % true (1)
Example: eps(single(1))   % single precision spacing
Differences from MATLAB: none known.

## error
Raise an error with a message and optional identifier.
Syntax: error(message)
Syntax: error(format, values...)
Syntax: error(identifier, format, values...)
Syntax: error(errStruct)
Stops the current block unless an enclosing try/catch handles the error. An identifier such as 'Demo:bad' must be followed by a message. A scalar error struct needs a character message and may include a character identifier. An empty message takes no action. Formatted messages are limited to 65536 UTF-8 bytes; unsupported formatting is rejected.
Example: try; error('A:b','v=%d',5); catch err; disp(err.message); end   % v=5
Differences from MATLAB: MatRust uses error structs with an empty stack; MException objects are not supported.

## exp
Exponential.
Syntax: exp(X)
Returns e raised to each element.
Complex input gives exp(x)*(cos(y) + i*sin(y)).
Example: exp(0)   % 1
Example: exp(1i*pi)   % -1 + 0i
Differences from MATLAB: double, logical and char 2-D arrays only.

## expm1
Accurate exp(x) - 1 for small x.
Syntax: expm1(X)
Returns exp(x) - 1 for each element without losing precision near zero.
Example: expm1(0)   % 0
Differences from MATLAB: Real double 2-D arrays, one output; complex and other classes are rejected.

## exportgraphics
Export a figure to an image or vector file.
Syntax: exportgraphics(gcf, 'plot.png')
Syntax: exportgraphics(gcf, 'plot.png', 'Resolution', 300)
Format comes from the extension: png, jpg, jpeg, tif, tiff, pdf or svg. Options: ContentType, Resolution, BackgroundColor (white, current or none). Call it as a statement.
Example: exportgraphics(gcf, 'plot.png', 'Resolution', 300)   % writes plot.png
Differences from MATLAB: Every format, SVG included, needs a build with the plot-export feature (on in the desktop build); without it every call errors. EPS is not accepted. Append, Colorspace, custom colors, sizes and padding are not supported; no output assignment.

## eye
Identity matrix.
Syntax: eye(n)
Syntax: eye(r, c)
Returns a matrix with ones on the main diagonal.
Syntax: eye(..., 'single')
Syntax: eye(..., 'like', single(1))
Example: eye(2, 'single')
Example: eye(2)   % [1 0; 0 1]
Class names 'single' and int8 through uint64 are accepted, as are typed integer/single 'like' prototypes.
Differences from MATLAB: No size vector.

## false
Logical zeros.
Syntax: false
Syntax: false(n)
Syntax: false(r, c) or false([r c])
false alone is a logical scalar; false(n) is n-by-n.
Example: false(1, 3)   % [false false false]
Differences from MATLAB: Two-dimensional output only.

## fft
Fast Fourier transform.
Syntax: fft(X)
Syntax: fft(X, n)
Syntax: fft(X, n, dim)
n pads or crops to length n; [] keeps the length. dim is 1 or 2. Length is at most 1048576.
Example: abs(fft([1 0 0 0]))   % [1 1 1 1]
Differences from MATLAB: Real, logical or complex 2-D input only; symmetry options are not supported.

## fieldnames
List a struct's field names.
Syntax: fieldnames(S)
Returns a column cell array of character names in field order. Works on scalar, array and empty structs. Other input classes and extra arguments are rejected.
Example: s=struct('a',1,'b','x'); names=fieldnames(s); disp(names{2})   % b
Differences from MATLAB: Cell display differs.

## figure
Create or select a figure.
Syntax: figure
Syntax: figure(n)
figure opens a new figure and makes it current. figure(n) selects figure n, creating it if needed (1 to 1000000). At most 16 figures can be open.
Example: figure(2)   % figure 2 is current
Differences from MATLAB: Figure handle input and property creation forms are only partly supported.

## fileparts
Folder, name and extension parts of a path.
Syntax: folder = fileparts(path); [folder, name, ext] = fileparts(path)
Returns the folder portion of a character path; it is empty if the path has no folder. name is the text after the last separator up to its last dot, ext that dot and what follows.
Example: [folder, name, ext] = fileparts('data/run1.csv')   % 'data', 'run1', '.csv'
Differences from MATLAB: Character row input only.

## filter
Digital filter.
Syntax: y = filter(b, a, x)
Syntax: y = filter(b, a, x, zi)
Syntax: [y, zf] = filter(b, a, x, zi, dim)
Filters x with numerator b and denominator a. zi is the initial state ([] means zero). Pass zf as the next call's zi to filter in blocks.
Example: y = filter(1, [1 -0.5], [1 0 0 0])   % [1 0.5 0.25 0.125]
Differences from MATLAB: dim must be 1 or 2. Complex coefficients, data and zi are supported.

## filtfilt
Zero-phase filtering.
Syntax: y = filtfilt(b, a, x)
Syntax: y = filtfilt(sos, g, x)
Filters forward and backward. A row vector x is filtered along the row; a matrix x is filtered column by column. x needs more than 3 times the filter order samples. The second form takes an SOS matrix with 1 to 64 sections (6 columns) and a gain g: a scalar or one value per section plus one. A single 6-element row is read as one SOS section when its 4th element is 1 and g has at most 2 elements.
Example: y = filtfilt([0.5 0.5], 1, [1 2 3 4 5 6])   % y = 1 2 3 4 5 6
Differences from MATLAB: Finite input only. Complex data and complex b, a are supported for the transfer-function form; SOS input must be real. SOS results not checked against MATLAB yet.

## filtic
Initial state for filter.
Syntax: zi = filtic(b, a, y)
Syntax: zi = filtic(b, a, y, x)
y and x hold past outputs and inputs, newest first. Returns a column vector.
Example: zi = filtic([1 1], [1 -0.5], 0, 0)   % 1-by-1 state
Differences from MATLAB: Real vectors only; a(1) must be 1; matrix histories are not supported.

## find
Indices of nonzero elements.
Syntax: find(A)
Returns linear indices. A row vector gives a row; any other shape gives a column.
Example: find([0 3 0 5])   % [2 4]
Differences from MATLAB: No n or direction arguments and no row/column index outputs.

## fir1
Window-based FIR filter design.
Syntax: b = fir1(order, Wn)
Syntax: b = fir1(order, Wn, type, window, 'noscale')
Wn is one or two normalized cutoffs. type, window (a vector) and 'scale' or 'noscale' are optional and must come in that order. Returns a row of order+1 coefficients.
Example: b = fir1(20, 0.3);   % 21 coefficients
Differences from MATLAB: Order must be a positive integer. Odd order for high or stop designs is raised by one, as in MATLAB.

## fix
Round toward zero.
Syntax: fix(X)
Truncates each real value toward zero. For complex values, truncates the real and imaginary parts separately. Logical inputs convert to double.
Example: fix([-2.5 -1.5 -0.5 0.5 1.5 2.5])   % [-2 -1 -0 0 1 2]
Differences from MATLAB: Only 2-D arrays are supported. Complex results with zero imaginary parts are real.

## flattopwin
Flat top window.
Syntax: flattopwin(n)
Syntax: flattopwin(n, 'periodic')
Returns an n-by-1 column vector. The default is symmetric.
Example: w = flattopwin(8);   % 8-by-1 column
Differences from MATLAB: Double output only.

## flip
Reverse the order of elements.
Syntax: flip(A)
Syntax: flip(A, dim)
Flips along the first non-singleton dimension unless dim is given.
Example: flip([1 2 3])   % [3 2 1]
Differences from MATLAB: Two-dimensional arrays only, including cell and struct arrays. Container results are limited to 65536 slots. Complex arrays keep complex storage and signed zeros.

## fliplr
Flip left to right.
Syntax: fliplr(A)
Reverses the order of columns. A character row is reversed too: fliplr('abc') gives 'cba'.
Example: fliplr([1 2 3])   % [3 2 1]
Differences from MATLAB: Two-dimensional arrays only, including character, cell and struct arrays. Complex arrays keep complex storage and signed zeros.

## flipud
Flip top to bottom.
Syntax: flipud(A)
Reverses the order of rows.
Example: flipud([1; 2])   % [2; 1]
Differences from MATLAB: Two-dimensional arrays only, including cell and struct arrays. Complex arrays keep complex storage and signed zeros.

## floor
Round toward negative infinity.
Syntax: floor(X)
Rounds each element down to an integer.
Example: floor(-1.5)   % -2
Differences from MATLAB: Real and complex double 2-D arrays. Complex results with zero imaginary parts are real.

## for
Repeat statements for each column of a range or matrix.
Syntax: for k = start:stop, statements, end
Syntax: for k = start:step:stop, statements, end
Syntax: for k = A, statements, end
k takes one column of the right side in each pass. For a range such as 1:3 it takes 1, then 2, then 3. A loop runs at most 1000000 steps per run. Use break to leave early.
Example: for k = 1:3, fprintf('%d ', k), end   % prints: 1 2 3
Differences from MATLAB: Not checked against MATLAB yet. The total of 1000000 execution steps per run is a MatRust limit.

## fprintf
Print formatted text in the console.
Syntax: fprintf(format)
Syntax: fprintf(format, A, ...)
Syntax: fprintf(fid, format, A, ...)
Formats like sprintf and prints the result. fid can be 1 or 2; both print to the same output (there is no separate error stream). Text without a final newline stays on the line, and the next output continues it. With an output, n = fprintf(...) returns the number of bytes written (UTF-8 length).
Example: fprintf('%s has %d\n', 'ab', 5)   % prints: ab has 5
Differences from MATLAB: Other file ids and file output are not supported yet.

## freqz
Plot the magnitude response of a filter.
Syntax: freqz(b, a)
Syntax: freqz(b, a, n)
Plots the response in dB at n points from 0 to Nyquist (default 512, at most 65536). It returns no values; use freqzmag to get numbers.
Example: freqz([1 1], 1, 256)   % plots magnitude in dB
Differences from MATLAB: Plot only. No outputs, no sampling-rate argument.

## freqzmag
Magnitude response of a filter.
Syntax: m = freqzmag(b, a)
Syntax: m = freqzmag(b, a, n)
Returns n linear magnitude values as a column (default 512). A bare call without assignment returns ans.
Example: m = freqzmag([1 1], 1, 256);   % 256-by-1 column
Differences from MATLAB: MATLAB has no freqzmag. Points 1 to 65536.

## fullfile
Build a file path.
Syntax: p = fullfile(part1, part2, ...)
Joins 1 to 64 character parts with the platform's separator.
Example: fullfile('data', 'run1.csv')   % 'data/run1.csv' (data\run1.csv on Windows)
Differences from MATLAB: Drive or device prefixes are rejected after the first part.

## function
Define a function.
Syntax: function name, statements, end
Syntax: function y = name(x), statements, end
Syntax: function [y, z] = name(a, b), statements, end
Put the definition in a .mr or .m file in the Current Folder, named like the function. Helper functions may follow in the same file. A function has its own workspace and can call up to 32 levels deep. Inside it, nargin and nargout give the number of inputs and outputs of the call.
A file twice.mr with these lines:
    function y = twice(x)
    y = 2 * x;
    end
then twice(4) returns 8. Help for such a function prints its leading comment lines.
Differences from MATLAB: Not checked against MATLAB yet. A file may hold only function definitions, so a definition cannot be typed in the command line together with other statements.

## gca
Handle of the current axes.
Syntax: h = gca
Returns the handle of the axes that plot commands use now. It takes no arguments. Use it with get and set.
Example: plot(1:3); h = gca; get(h, 'Type')   % 'axes'
Differences from MATLAB: Not checked against MATLAB yet.

## gcf
Handle of the current figure.
Syntax: f = gcf
Returns a handle for the current figure, creating one if needed. Use it with saveas, savefig or exportgraphics.
Example: saveas(gcf, 'plot.svg')   % saves the current figure
Differences from MATLAB: Takes no arguments.

## get
Read a property of a graphics object.
Syntax: v = get(h, 'PropertyName')
h is a handle from gca, gcf, figure, axes, plot or patch. One property name per call. Axes: Type, Parent, XLim, YLim, XScale, YScale, XGrid, YGrid, Position, NextPlot, Box, Color, XColor, YColor. Figure: Name, Number, Theme, ThemeMode, ColorMode, Color, CurrentAxes. Lines: Type, Parent, DisplayName, Visible, LineWidth. Reading LineWidth of a line errors until LineWidth has been set on it. For bar and patch objects LineWidth is 0.5 by default.
Example: plot(1:3); get(gca, 'XLim')   % [1 3]
Differences from MATLAB: Not checked against MATLAB yet. get(h) without a property name and get(0) are not supported yet; other properties are rejected with an error.

## getfield
Read a field from a scalar struct.
Syntax: getfield(S, field)
Syntax: getfield(S, field1, field2, ...)
Follows the given character field names through nested scalar structs and returns the final value. Missing fields raise an error.
Example: s=struct('a',1,'b','x'); getfield(s,'b')   % 'x'
Differences from MATLAB: Cell index arguments and nonscalar structs are not supported.

## grid
Show or hide grid lines.
Syntax: grid on
Syntax: grid off
Syntax: grid
With no argument it toggles the grid of the current axes.
Example: grid on   % grid lines shown
Differences from MATLAB: Only on, off and toggle; no minor-grid options.

## grpdelay
Group delay of a filter.
Syntax: d = grpdelay(b, a)
Syntax: d = grpdelay(b, a, n)
Returns n values in samples as a column (default 512, at most 65536). Undefined points are NaN. A bare call without assignment plots it.
Example: d = grpdelay([1 1], 1, 128);   % 128-by-1 column
Differences from MATLAB: One output; no sampling-rate argument.

## hamming
Hamming window.
Syntax: hamming(n)
Syntax: hamming(n, 'periodic')
Returns an n-by-1 column vector. The default is symmetric.
Example: hamming(5)'   % [0.08 0.54 1 0.54 0.08]
Differences from MATLAB: Double output only.

## hann
Hann window.
Syntax: hann(n)
Syntax: hann(n, 'periodic')
Returns an n-by-1 column vector. The default is symmetric.
Example: hann(5)'   % [0 0.5 1 0.5 0]
Differences from MATLAB: Double output only.

## help
Show help for a function, or the overview.
Syntax: help
Syntax: help name
Syntax: doc name
help alone prints the overview of what MatRust supports.
help name prints the page for a built-in function. For your own .mr or .m
function in the Current Folder it prints the comment lines at the top of the
file or right after the function line, without the % signs.
doc name is the same as help name.
Example: help plot
Differences from MATLAB: doc does not open a browser. Help for folders,
packages, methods and operators is not supported yet.

## hold
Keep or replace existing plots.
Syntax: hold on
Syntax: hold off
Syntax: hold
With hold on, new plot calls add to the current axes. With no argument it toggles the state.
Example: hold on   % later plots are added
Differences from MATLAB: Only on, off and toggle.

## hypot
Length from two perpendicular components.
Syntax: hypot(X, Y)
Computes sqrt(X.^2 + Y.^2) without forming the squares directly. Inputs must have the same size, or one must be scalar. Real numeric and logical inputs are accepted; complex input is rejected.
Example: hypot(3,4)   % 5
Differences from MATLAB: none known.

## i
Imaginary unit.
Syntax: i
Syntax: 3 + 2i
Both i and the literal 2i make complex values. A variable named i takes precedence.
Example: z = 3 + 2*i   % 3 + 2i
Differences from MATLAB: none known.

## idivide
Integer division with a chosen rounding.
Syntax: idivide(A, B), idivide(A, B, OPT)
At least one input is an integer array; the other is the same class or a scalar double. OPT is 'fix' (the default), 'round', 'floor' or 'ceil'. Division by zero saturates.
Example: q = idivide(int32(7), int32(2), 'floor')
Differences from MATLAB: none known.

## if
Run statements only when a condition is true.
Syntax: if condition, statements, end
Syntax: if condition, statements, else, statements, end
Syntax: if condition, statements, elseif condition, statements, else, statements, end
The condition is true when it is nonempty and all its elements are nonzero. Use && and || to combine scalar conditions. Write each statement block on its own lines in a script.
Example: x = 3; if x > 2, disp('big'), else, disp('small'), end   % prints: big
Differences from MATLAB: Not checked against MATLAB yet.

## ifft
Inverse fast Fourier transform.
Syntax: ifft(X)
Syntax: ifft(X, n)
Syntax: ifft(X, n, dim)
Same arguments as fft. The result is complex unless it is real by construction.
Example: real(ifft(fft([1 2 3])))   % [1 2 3]
Differences from MATLAB: Real, logical or complex 2-D input only; symmetry options are not supported.

## imag
Imaginary part.
Syntax: imag(Z)
Returns the imaginary part as a real array. Real input gives zeros.
Example: imag(3+4i)   % 4
Differences from MATLAB: none known.

## impz
Impulse response of a filter.
Syntax: h = impz(b, a, n)
Returns the first n samples (1 to 65536) as a column. n is required. A bare call without assignment plots it.
Example: h = impz([1 1], 1, 8)   % [1; 1; 0; 0; 0; 0; 0; 0]
Differences from MATLAB: n is required; one output.

## Inf
Positive infinity.
Syntax: Inf
Syntax: x = -Inf
Result of 1/0. Use isinf to test for it and isfinite to exclude it. inf is the same.
Example: 1/0   % inf
Differences from MATLAB: Not checked against MATLAB yet. Only the form without arguments; Inf(n) and Inf(m, n) are not supported yet.

## inf
Positive infinity. Same as Inf.
Syntax: inf
Example: isinf(inf)   % true
Differences from MATLAB: Not checked against MATLAB yet. Only the form without arguments.

## int16
Convert to signed 16-bit integers.
Syntax: int16(X)
Rounds half away from zero and saturates at intmin('int16')..intmax('int16'); NaN becomes 0. Each element takes 2 bytes.
Arithmetic with the same class or a scalar double stays int16; other mixes are errors.
Example: x = int16([2.5 -3.5 1000])
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## int2str
Round numbers and convert them to text.
Syntax: s = int2str(x)
Rounds x to the nearest integer (halves away from zero) and formats it like num2str. A matrix gives one text row per matrix row. Character input is formatted as its character codes: int2str('ab') gives '97  98'.
Example: int2str(2.7)   % '3'
Differences from MATLAB: none known. Character input works too, for example int2str('ab') and int2str(['ab';'cd']).

## int32
Convert to signed 32-bit integers.
Syntax: int32(X)
Rounds half away from zero and saturates at intmin('int32')..intmax('int32'); NaN becomes 0. Each element takes 4 bytes.
Arithmetic with the same class or a scalar double stays int32; other mixes are errors.
Example: x = int32([2.5 -3.5 1000])
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## int64
Convert to signed 64-bit integers.
Syntax: int64(X)
Rounds half away from zero and saturates at intmin('int64')..intmax('int64'); NaN becomes 0. Each element takes 8 bytes.
Arithmetic with the same class or a scalar double stays int64; other mixes are errors.
Example: x = int64([2.5 -3.5 1000])
int64 values stay exact 64-bit integers (an integer literal above 2^53 inside int64(...) is kept exact).
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## int8
Convert to signed 8-bit integers.
Syntax: int8(X)
Rounds half away from zero and saturates at intmin('int8')..intmax('int8'); NaN becomes 0. Each element takes 1 byte.
Arithmetic with the same class or a scalar double stays int8; other mixes are errors.
Example: x = int8([2.5 -3.5 1000])
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## interp1
One-dimensional interpolation.
Syntax: vq = interp1(x, v, xq)
Syntax: vq = interp1(x, v, xq, method, fill)
method is 'linear' (default) or 'nearest'. Queries outside x give NaN, or the scalar fill. The output has the shape of xq.
Example: interp1([1 2 3], [10 20 30], 2.5)   % 25
Differences from MATLAB: Finite real vectors with distinct x. No implicit x, matrix samples, extrapolation or other methods.

## intersect
Values common to two sets.
Syntax: C = intersect(A, B)
Syntax: [C, ia, ib] = intersect(A, B, 'stable')
Result is sorted by default; 'stable' keeps the order of A. ia and ib are first indices.
Example: intersect([1 2 3], [2 3 4])   % [2 3]
Differences from MATLAB: Numeric inputs support real/logical vectors and shaped empties with 'sorted' or 'stable'. Character/cellstr forms also support 'rows' and 'legacy'; 'rows' warns and is ignored for cell inputs.

## intmax
Largest value of an integer class.
Syntax: intmax, intmax(CLASSNAME)
CLASSNAME is 'int8', 'uint8', 'int16', 'uint16', 'int32' (the default), 'uint32', 'int64' or 'uint64'.
Example: x = intmax('int8')
Differences from MATLAB: the 'like' form is not supported yet.

## intmin
Smallest value of an integer class.
Syntax: intmin, intmin(CLASSNAME)
CLASSNAME is an integer class name; the default is 'int32'.
Example: x = intmin('int16')
Differences from MATLAB: the 'like' form is not supported yet.

## inv
Matrix inverse.
Syntax: inv(A)
Returns the inverse of a square matrix. A singular matrix is an error. For solving A*x = b use A\b.
Example: inv([2 0; 0 4])   % [0.5 0; 0 0.25]
Differences from MATLAB: real input must be double with finite values, and a singular real matrix raises an error, not a warning. Complex double input works as in MATLAB: a singular matrix warns "Matrix is singular to working precision." and gives Inf, an ill-conditioned one warns with its RCOND estimate.

## isa
True if a value belongs to a class.
Syntax: isa(A, CLASSNAME)
CLASSNAME is a class name such as 'double' or 'int8', or a category: 'numeric', 'float' or 'integer'.
Example: tf = isa(int8(1), 'integer')
Differences from MATLAB: user-defined classes do not exist in MatRust.

## iscell
True for a cell array.
Syntax: tf = iscell(A)
Example: iscell({1})   % true
Differences from MATLAB: Not checked against MATLAB yet.

## iscellstr
True for a cell array of text.
Syntax: tf = iscellstr(c)
Returns true if c is a cell array and every element is a character array of any shape (a row or a matrix). An empty cell array gives true.
Example: iscellstr({'a','b'})   % true
Differences from MATLAB: none known.

## ischar
True for text.
Syntax: tf = ischar(A)
Returns true for a character array, including the empty text ''.
Example: ischar('abc')   % true
Differences from MATLAB: Not checked against MATLAB yet.

## iscolumn
True for column vectors.
Syntax: iscolumn(A)
Returns true if A has one column.
Example: iscolumn([1; 2])   % true
Differences from MATLAB: none known.

## isempty
True if an array has no elements.
Syntax: isempty(A)
Returns true if any dimension is zero.
Example: isempty([])   % true
Differences from MATLAB: none known.

## isfield
Test whether a struct has named fields.
Syntax: isfield(S, name)
Syntax: isfield(S, names)
A character name gives a logical scalar. A cell array of names gives a logical array of the same shape. Names must match exactly. A non-struct input, or a cell entry that is not a character row, gives false.
Example: isfield(struct('a',1),{'a','z'})   % [true false]
Differences from MATLAB: Logical display differs.

## isfinite
True for finite elements.
Syntax: isfinite(A)
Returns a logical array. NaN and Inf give false.
Example: isfinite([1 Inf NaN])   % [true false false]
Differences from MATLAB: none known.

## isfloat
True for floating-point arrays.
Syntax: isfloat(A)
True for double and single arrays (real or complex); false for integer, logical and char arrays.
Example: tf = isfloat(single(1))
Differences from MATLAB: none known.

## isgraphics
True for valid graphics handles.
Syntax: tf = isgraphics(h)
Returns true where h holds a handle to an object that still exists, for example the handle of gca or of a line. A handle becomes invalid after its figure is closed.
Example: plot(1:3); h = gca; isgraphics(h)   % true
Differences from MATLAB: Not checked against MATLAB yet. Only handles made by MatRust are accepted; a number such as isgraphics(1) is rejected. No type argument.

## isinf
True for infinite elements.
Syntax: isinf(A)
Returns a logical array of the same size.
Example: isinf([1 Inf NaN])   % [false true false]
Differences from MATLAB: none known.

## isinteger
True for integer arrays.
Syntax: isinteger(A)
True for int8, uint8, int16, uint16, int32, uint32, int64 and uint64 arrays; false for double values even when they are whole.
Example: tf = isinteger(int8(1))
Differences from MATLAB: none known.

## islogical
True for logical arrays.
Syntax: islogical(A)
Returns true if A has logical type.
Example: islogical(true)   % true
Differences from MATLAB: none known.

## ismatrix
True for 2-D arrays.
Syntax: ismatrix(A)
Always true for arrays in MatRust, which are 2-D.
Example: ismatrix(5)   % true
Differences from MATLAB: none known.

## ismember
Test set membership.
Syntax: tf = ismember(A, B)
Syntax: [tf, loc] = ismember(A, B)
tf marks elements of A found in B. loc is the first index in B, or 0. NaN never matches.
Example: [tf, loc] = ismember([1 7], [5 6 1])   % tf = [true false], loc = [3 0]
Differences from MATLAB: Numeric inputs support real/logical vectors and shaped empties without flags. Character/cellstr forms support 'rows' and 'legacy'; 'rows' warns and is ignored for cell inputs.

## isnan
True for NaN elements.
Syntax: isnan(A)
Returns a logical array of the same size.
Example: isnan([1 NaN])   % [false true]
Differences from MATLAB: none known.

## isnumeric
True for numeric arrays.
Syntax: isnumeric(A)
True for real and complex arrays. False for logical arrays.
Example: isnumeric(5)   % true
Differences from MATLAB: none known.

## isreal
True if storage is real.
Syntax: isreal(A)
Complex storage gives false even when the imaginary part is zero, for example complex(1,0).
Example: isreal(1+2i)   % false
Differences from MATLAB: none known.

## isrow
True for row vectors.
Syntax: isrow(A)
Returns true if A has one row.
Example: isrow([1 2 3])   % true
Differences from MATLAB: none known.

## isscalar
True for 1-by-1 arrays.
Syntax: isscalar(A)
Returns true if A has exactly one element.
Example: isscalar(5)   % true
Differences from MATLAB: none known.

## isstruct
Test whether a value is a struct.
Syntax: isstruct(X)
Returns a logical scalar. Scalar, array and empty structs all return true; other classes return false.
Example: isstruct(struct('a',1))   % true
Differences from MATLAB: Logical display differs.

## isvector
True for vectors.
Syntax: isvector(A)
Returns true if A has one row or one column.
Example: isvector([1 2 3])   % true
Differences from MATLAB: Uses the simple rule rows == 1 or columns == 1.

## j
Imaginary unit.
Syntax: j
Syntax: 3 + 2j
Same as i. A variable named j takes precedence.
Example: z = 1 + j   % 1 + 1i
Differences from MATLAB: none known.

## kaiser
Kaiser window.
Syntax: kaiser(n)
Syntax: kaiser(n, beta)
Returns an n-by-1 symmetric column. The default beta is 0.5; [] also means 0.5.
Example: w = kaiser(51, 8);   % 51-by-1 column
Differences from MATLAB: Double output; a trailing 'double' is accepted. No single or periodic options.

## kron
Kronecker product of two arrays.
Syntax: kron(A, B)
Replaces each element of A with that element times B. Output has size (rows(A)*rows(B))-by-(cols(A)*cols(B)). Real, complex and logical inputs are accepted; logical inputs produce double output. The result must fit the matrix and statement size limits.
Example: kron([1 2],[1;1])   % [1 2; 1 2]
Differences from MATLAB: Only full 2-D arrays are supported.

## legend
Show a legend.
Syntax: legend
Syntax: legend('sin', 'cos')
Syntax: legend('sin', 'cos', 'Location', 'northeast')
Labels are given in series order, one per series. 'on' and 'off' show or hide it. Location accepts the eight compass points, outside variants, 'best' and 'bestoutside'.
Example: plot(1:3,1:3); hold on; plot(1:3,[3 2 1]); legend('sin','cos')   % labels both series
Differences from MATLAB: The label count must equal the series count. Some graphics-handle forms are limited.

## length
Length of the largest dimension.
Syntax: length(A)
Returns the larger of rows and columns, or 0 for an empty array.
Example: length([1 2 3; 4 5 6])   % 3
Differences from MATLAB: none known.

## linspace
Evenly spaced points.
Syntax: linspace(a, b)
Syntax: linspace(a, b, n)
Returns a row of n points from a to b. The default n is 100.
Example: linspace(0, 1, 5)   % [0 0.25 0.5 0.75 1]
Differences from MATLAB: Real scalar endpoints only.

## load
Load variables from a MAT file.
Syntax: load('data.mat')
Syntax: load('data.mat', 'A', 'B')
Syntax: load data.mat A
Without names it loads every variable in the file. The default file is matlab.mat. Files are found in the Current Folder.
Example: A=5; save('data.mat','A'); clear A; load('data.mat','A'); A   % 5
Differences from MATLAB: Statement only; no output assignment. MAT files only: ASCII files are not supported (-mat only lets a MAT file have another extension). No wildcards or regular expressions, no structures.

## log
Natural logarithm.
Syntax: log(X)
Applies the natural log to each element.
Negative real input and complex input give the principal complex logarithm.
Example: log(1)   % 0
Example: log(-1)   % 0 + 3.1416i
Differences from MATLAB: double, logical and char 2-D arrays only.

## log10
Base-10 logarithm.
Syntax: log10(X)
Applies log base 10 to each element. Negative real values give complex results.
Example: log10(100)   % 2
Differences from MATLAB: Double real or complex 2-D arrays only.

## log1p
Accurate log(1 + x) for small x.
Syntax: log1p(X)
Returns log(1 + x) without losing precision near zero.
Example: log1p(0)   % 0
Differences from MATLAB: Real double 2-D arrays, one output. Values below -1 are rejected.

## log2
Base-2 logarithm.
Syntax: log2(X)
Syntax: [F, E] = log2(X)
The two-output form splits x into F and E with x = F * 2^E and 0.5 <= abs(F) < 1.
Example: [f, e] = log2(8)   % f = 0.5, e = 4
Differences from MATLAB: Double arrays only. The two-output form needs real input; complex input is rejected.

## logical
Convert to logical.
Syntax: logical(X)
Nonzero values become true and zeros become false.
Example: logical([2 0])   % [true false]
Differences from MATLAB: none known.

## loglog
Plot with logarithmic axes.
Syntax: loglog(x, y)
Syntax: loglog(Z)
Both axes use log scale. A single complex array plots real(Z) against imag(Z).
Example: loglog([1 10 100], [1 100 10000])   % straight line on log axes
Differences from MATLAB: Properties are trailing name/value pairs such as 'LineWidth', 2.

## logspace
Logarithmically spaced row vector.
Syntax: logspace(a, b)
Syntax: logspace(a, b, n)
Returns n points from 10^a to 10^b; n defaults to 50. When b equals pi, the endpoint is pi instead of 10^pi. With one point it returns the endpoint. Fractional counts round down, nonpositive counts give an empty row, and NaN counts error. Inputs must be real scalars, and the output must fit the matrix and statement limits.
Example: logspace(0,2,5)   % [1 3.162278 10 31.622777 100]
Differences from MATLAB: Complex endpoints are not supported.

## lower
Convert text to lower case.
Syntax: s = lower(s)
Syntax: c = lower(c)
A cell array of text gives a cell array. Numeric input is returned unchanged.
Example: lower('ABC')   % 'abc'
Differences from MATLAB: Character matrices are not supported yet.

## mat2str
Convert a matrix to text that evaluates to the matrix.
Syntax: s = mat2str(A)
Syntax: s = mat2str(A, precision)
Rows are separated by ; and elements by a space. Logical values print as true and false, text in single quotes. A character matrix gives one quoted text per row, such as ['ab';'cd']. precision is the number of significant digits (default 15).
Example: mat2str([1 2; 3 4.5])   % '[1 2;3 4.5]'
Differences from MATLAB: The 'class' option is not supported yet.

## max
Largest element.
Syntax: max(A)
Syntax: max(A, B)
max(A) gives the maximum of each column, or one value for a row vector. max(A, B) compares element by element; one input may be a scalar.
Example: max([3 9 2])   % 9
Differences from MATLAB: One output only (no index). No dim, 'omitnan' or 'all' options. Real arrays. Complex input orders by abs, then angle.

## mean
Average value.
Syntax: mean(A)
Returns the mean of each column, or one value for a row vector.
Example: mean([1 2 3 4])   % 2.5
Differences from MATLAB: No 'all', type or NaN options. The mean of [] is NaN. Complex input divides the narrowed complex sum by the count, as MATLAB does.

## median
Median value.
Syntax: median(A)
Syntax: median(A, dim)
Syntax: median(A, 'all', 'omitnan')
dim is a positive integer or 'all'. 'omitnan' or 'includenan' may follow.
Example: median([3 1 2])   % 2
Differences from MATLAB: Real and complex double arrays. Logical input, weights and vecdim are not supported. Complex values order by abs, then angle in (-pi, pi], NaN last; a lane with NaN gives NaN.

## meshgrid
Create a 2-D grid with x varying across columns.
Syntax: [X,Y] = meshgrid(x, y)
Syntax: [X,Y] = meshgrid(x)
Syntax: X = meshgrid(x, y)
Returns numel(y)-by-numel(x) arrays. Each row of X repeats x, and each column of Y repeats y. One input uses x for both axes. One output returns X only. Inputs are numeric or logical (complex allowed); matrix inputs are read in column order.
Example: [X,Y]=meshgrid(1:3,1:2); X   % [1 2 3; 1 2 3]
Differences from MATLAB: A grid whose imaginary parts are all zero is real. The 3-D form is not supported.

## mfilename
Name of the running script.
Syntax: mfilename()
Syntax: mfilename('fullpath')
The first form returns the file name without extension; the second includes the folder. It returns an empty text if the script is unsaved.
Example: name = mfilename()   % 'myscript' when run from myscript.m; '' at the prompt
Differences from MATLAB: Does not change the Current Folder.

## min
Smallest element.
Syntax: min(A)
Syntax: min(A, B)
min(A) gives the minimum of each column, or one value for a row vector. min(A, B) compares element by element; one input may be a scalar.
Example: min([3 9 2])   % 2
Differences from MATLAB: One output only (no index). No dim, 'omitnan' or 'all' options. Real arrays. Complex input orders by abs, then angle.

## mod
Remainder using division rounded toward negative infinity.
Syntax: mod(A, B)
For finite inputs and nonzero B, the nonzero remainder has the sign of B. mod(A,0) returns A. Other nonfinite inputs produce NaN. Inputs must have the same size, or one must be scalar. Real numeric and logical inputs are accepted; complex input is rejected.
Example: mod([-7 7],3)   % [2 1]
Differences from MATLAB: none known, also for infinite divisors and the sign of zero results for exact multiples.

## NaN
Not a number.
Syntax: NaN
Result of undefined operations such as Inf - Inf. NaN is not equal to anything, including itself. Use isnan to test for it. nan is the same.
Example: isnan(Inf - Inf)   % true
Differences from MATLAB: Not checked against MATLAB yet. Only the form without arguments; NaN(n) and NaN(m, n) are not supported yet.

## nan
Not a number. Same as NaN.
Syntax: nan
Example: nan == nan   % false
Differences from MATLAB: Not checked against MATLAB yet. Only the form without arguments.

## nargin
Number of input arguments of a function.
Syntax: nargin
Inside a function it returns how many inputs the caller passed.
Example: n = nargin   % inside a function called with 2 inputs: 2
Differences from MATLAB: Only the zero-argument form inside a function; nargin('name') is not supported.

## nargout
Number of output arguments of a function.
Syntax: nargout
Inside a function it returns how many outputs the caller requested.
Example: n = nargout   % inside a function called as [a,b] = f(): 2
Differences from MATLAB: Only the zero-argument form inside a function.

## ndgrid
Create a 2-D grid with x varying down rows.
Syntax: [X,Y] = ndgrid(x, y)
Syntax: [X,Y] = ndgrid(x)
Syntax: X = ndgrid(x)
Two outputs have size numel(x)-by-numel(y). Each column of X repeats x, and each row of Y repeats y. With one input and two outputs, y equals x. With one input and one output, x is returned as a column. Real numeric and logical inputs are accepted; matrix inputs are read in column order.
Example: [X,Y]=ndgrid(1:3,1:2); X   % [1 1; 2 2; 3 3]
Differences from MATLAB: The 3-D form and complex inputs are not supported.

## ndims
Number of dimensions.
Syntax: ndims(A)
Always returns 2 for MatRust arrays.
Example: ndims([1 2; 3 4])   % 2
Differences from MATLAB: none known.

## nextpow2
Exponent of the next power of two.
Syntax: nextpow2(X)
Returns the smallest p with 2^p >= abs(x), for each element.
Example: nextpow2(100)   % 7
Differences from MATLAB: Real double 2-D arrays, one output.

## nexttile
Select the next tile of a tiled layout.
Syntax: nexttile
Syntax: nexttile(index)
Syntax: nexttile(index, [rows cols])
nexttile([rows cols]) also works. It moves to the next free tile, or to the tile given by index (1 to 256).
Example: nexttile(2)   % tile 2 becomes current
Differences from MATLAB: No layout handle argument.

## nnz
Count nonzero elements.
Syntax: nnz(A)
Returns the number of nonzero elements.
Example: nnz([1 0 2])   % 2
Differences from MATLAB: none known, also for complex input.

## norm
Vector 2-norm.
Syntax: norm(v)
Returns the Euclidean length of a vector.
Example: norm([3 4])   % 5
Differences from MATLAB: Vectors only, no p argument. Matrices are rejected.

## not
Logical negation.
Syntax: not(A)
Same as ~A.
Example: not([1 0])   % [false true]
Differences from MATLAB: none known.

## num2cell
Put each array element in its own cell.
Syntax: c = num2cell(A)
Returns a cell array of the same size as A. Numeric and logical input only, at most 65536 elements.
Example: c = num2cell([1 2]); c{2}   % 2
Differences from MATLAB: Character input is not supported yet.

## num2str
Convert numbers to text.
Syntax: s = num2str(x)
Syntax: s = num2str(x, precision)
Syntax: s = num2str(x, format)
Integers print in full. Otherwise every element gets floor(log10(m)) + 5 significant digits, where m is the largest finite abs(x), kept between 5 and 16; so pi gives '3.1416'. A matrix gives one text row per matrix row. precision is the number of significant digits. format is a sprintf format such as '%8.3f'. Character input is returned unchanged. Complex and logical input is accepted.
Example: num2str(pi, 8)   % '3.1415927'
Differences from MATLAB: Complex input with a format is not supported yet.

## numel
Number of elements.
Syntax: numel(A)
Returns rows times columns.
Example: numel([1 2; 3 4])   % 4
Differences from MATLAB: none known.

## numfields
Count a struct's fields.
Syntax: numfields(S)
Returns a double scalar counting the field names, independently of the number of struct elements. Empty structs can still have fields. Exactly one struct input is required.
Example: numfields(struct('a',1,'b','x'))   % 2
Differences from MATLAB: Not checked against MATLAB yet.

## nuttallwin
Nuttall minimum 4-term window.
Syntax: nuttallwin(n)
Syntax: nuttallwin(n, 'periodic')
Returns an n-by-1 column vector. The default is symmetric.
Example: w = nuttallwin(64);   % 64-by-1 column
Differences from MATLAB: Double output only.

## ones
Array of ones.
Syntax: ones(n)
Syntax: ones(r, c)
Syntax: ones([r c])
ones(n) is n-by-n. A size vector such as size(A) is accepted.
Syntax: ones(..., 'single')
Syntax: ones(..., 'like', single(1))
Example: ones(2, 'single')
Example: ones(2, 3)   % 2-by-3 array of ones
Class names 'single' and int8 through uint64 are accepted, as are typed integer/single 'like' prototypes.
Differences from MATLAB: Two-dimensional output only.

## openfig
Open a saved figure.
Syntax: f = openfig('plot.mrfig')
Syntax: openfig('plot.fig', 'new')
Loads .fig, .mrfig or .mfig files from the Current Folder into a new figure and returns its handle.
Example: plot(1:3); savefig('plot.mrfig'); f = openfig('plot.mrfig')   % figure opens, f is its handle
Differences from MATLAB: Only the filename and the 'new' option. At most 16 figures can be open.

## or
Element-wise logical OR.
Syntax: or(A, B)
Same as A | B. Inputs are converted to logical.
Example: or([1 0], [0 0])   % [true false]
Differences from MATLAB: Exactly two arguments.

## patch
Draw filled polygons.
Syntax: p = patch('XData', x, 'YData', y, 'FaceColor', color, 'EdgeColor', edge)
Syntax: p = patch(ax, 'XData', x, 'YData', y, 'FaceColor', color, 'EdgeColor', edge)
Draws the polygon with corners (x, y) in the given axes, or in the current axes. color and edge are RGB rows such as [1 0 0]; edge can also be 'none'. EdgeColor is required. Up to 16 property and value pairs.
Example: a = gca; p = patch(a, 'XData', [0;1;0], 'YData', [0;0;1], 'FaceColor', [1 0 0], 'EdgeColor', 'none'); get(p, 'Type')   % 'patch'
Differences from MATLAB: Not checked against MATLAB yet. Only the name-value form is supported; patch(x, y, c) is rejected, and so is a call without EdgeColor.

## phasez
Phase response of a filter.
Syntax: p = phasez(b, a)
Syntax: p = phasez(b, a, n)
Returns n phase values in radians as a column (default 512, at most 65536). A bare call without assignment plots it.
Example: p = phasez([1 1], 1, 8);   % 8-by-1 column
Differences from MATLAB: One output; no sampling-rate argument.

## pi
The number pi.
Syntax: pi
A constant, 3.14159265358979.
Example: 2*pi   % 6.2832
Differences from MATLAB: Written without parentheses.

## plot
2-D line plot.
Syntax: plot(y)
Syntax: plot(x, y)
Syntax: plot(x, y, style)
Syntax: plot(x1, y1, style1, x2, y2, style2)
Syntax: plot(x, y, 'LineWidth', 2, 'Color', 'red')
Draws y against x in the current figure. With one input, x is 1, 2, 3, ...
style is a short text such as 'r--o' (color, line style, marker).
Name-value pairs at the end apply to the new lines.
Use hold on to add lines instead of replacing them.
Complex Z in plot(Z) draws real(Z) against imag(Z).
Example: x = 0:0.1:10; plot(x, sin(x), 'r--')
Differences from MATLAB: up to 16 figures and 64 lines per figure; many line
properties are not supported yet.

## polyfit
Fit a polynomial to data by least squares.
Syntax: p = polyfit(x, y, degree)
Returns a row of coefficients in descending powers. x and y must be finite real vectors of equal length; logical inputs convert to double. Degree must be an integer from 0 to 4096, and the fitting matrix must fit the statement limit. Underdetermined and rank-deficient systems use a basic solution with dependent coefficients set to zero.
Example: round(polyfit(1:5,[1 4 9 16 25],2))   % [1 -0 0]
Differences from MATLAB: Ill-conditioned and rank-deficient coefficients can differ, and MATLAB's conditioning warnings are not printed. Underdetermined fits (fewer points than coefficients) work. The S and mu outputs are not supported. Complex x or y is fitted by Householder QR of the Vandermonde matrix; the last digits can differ from MATLAB's MKL QR.

## polyval
Evaluate a polynomial.
Syntax: y = polyval(p, x)
p holds coefficients from the highest power down (1 to 4096 values). Uses Horner's method.
Example: polyval([1 2 3], 2)   % 11
Differences from MATLAB: Finite inputs, one output. No scaling or error estimates. Complex p or x is evaluated by the same recurrence; a result with zero imaginary parts is real.

## pow2
Power of two.
Syntax: pow2(X)
Syntax: pow2(F, E)
pow2(X) gives 2.^X. pow2(F,E) gives F.*2.^E, with E truncated toward zero.
Example: pow2(3)   % 8
Differences from MATLAB: Real double 2-D arrays, one output. F and E must have equal shapes or one must be a scalar.

## prod
Product along an array dimension.
Syntax: prod(A)
Syntax: prod(A, dim)
Multiplies along the first nonsingleton dimension by default: across a row vector, or down matrix columns. Dim 1 works down rows; dim 2 works across columns; dim above 2 leaves the values unchanged. Empty products are 1. Real, complex and logical inputs are accepted; logical inputs convert to double.
Example: prod([1 2;3 4],2)   % [2; 12]
Differences from MATLAB: Output-class and NaN-handling options are not supported. A complex product with zero imaginary parts is real.

## rand
Uniform random numbers between zero and one.
Syntax: rand()
Syntax: rand(n)
Syntax: rand(m, n)
Syntax: rand([m n])
No size gives a scalar; one scalar size gives a square matrix. Values lie strictly between 0 and 1 and fill columns first. rand, randn and randi share one generator per engine session. Use rng to set the seed. An optional 'double' or 'single' class is accepted. Sizes must fit the matrix and statement limits; higher dimensions must be singleton.
Syntax: rand(..., 'single')
Syntax: rand(..., 'like', single(1))
Example: rand(2, 'single')
Example: rng(0); rand(1,3)   % [0.814724 0.905792 0.126987]
Differences from MATLAB: Legacy generator-state forms are not supported.

## randi
Uniform random integers in an inclusive range.
Syntax: randi(imax)
Syntax: randi(imax, m, n)
Syntax: randi([imin imax], [m n])
The scalar limit means 1 through imax. A pair gives inclusive lower and upper limits. Limits must be finite integers with absolute value below 2^53 and imin <= imax; a scalar imax must be at least 1. Sizes follow rand: no size gives a scalar and one scalar size gives a square matrix. Output is double, with optional 'double' accepted; higher dimensions must be singleton.
Example: rng(0); randi(10,1,4)   % [9 10 2 10]
Differences from MATLAB: Other output classes are not supported.

## randn
Normally distributed random numbers.
Syntax: randn()
Syntax: randn(n)
Syntax: randn(m, n)
Syntax: randn([m n])
Returns double values from a standard normal distribution. Sizes follow rand: no size gives a scalar and one scalar size gives a square matrix. Values fill columns first and share the session generator with rand and randi. Use rng to set the seed. An optional 'double' class is accepted; higher dimensions must be singleton, and sizes must fit the matrix and statement limits.
Example: rng(0); randn(1,3)   % [0.537667 1.833885 -2.258847]
Differences from MATLAB: Seeded normal values differ; do not expect identical MATLAB sequences. Other output classes and legacy generator-state forms are not supported.

## rank
Numerical rank from singular values.
Syntax: rank(A)
Syntax: rank(A, tolerance)
Counts singular values strictly greater than tolerance. The default is max(size(A))*eps(s), where s is the largest singular value. Empty input has rank zero. A must be finite and may be complex; logical input converts to double. An explicit tolerance must be a real numeric or logical scalar.
Example: rank([1 2 3;4 5 6;7 8 9])   % 2
Differences from MATLAB: Very small tolerances can give a different rank because the computed singular values differ.

## readmatrix
Read a numeric matrix from a text file.
Syntax: A = readmatrix('data.csv')
Syntax: A = readmatrix('data.csv', 'Delimiter', ';')
Reads .csv, .txt or .dat files with numbers separated by comma, semicolon or tab. 'FileType', 'text' allows other extensions.
Example: writematrix([1 2;3 4],'data.csv'); A = readmatrix('data.csv')   % 2-by-2 numeric matrix
Differences from MATLAB: Strictly numeric text only. No spreadsheets, headers, ranges or missing-data options.

## real
Real part.
Syntax: real(Z)
Returns the real part as a real array. Real input is returned unchanged.
Example: real(3+4i)   % 3
Differences from MATLAB: none known.

## rectwin
Rectangular window.
Syntax: rectwin(n)
Returns an n-by-1 column of ones.
Example: rectwin(4)'   % [1 1 1 1]
Differences from MATLAB: No sampling option; double output only.

## rem
Remainder using division rounded toward zero.
Syntax: rem(A, B)
For finite inputs and nonzero B, a nonzero remainder has the sign of A. A zero divisor or a nonfinite input gives NaN. Inputs must have the same size, or one must be scalar. Real numeric and logical inputs are accepted; complex input is rejected.
Example: rem([-7 7],3)   % [-1 1]
Differences from MATLAB: none known, also for infinite divisors.

## repmat
Repeat an array.
Syntax: repmat(A, n)
Syntax: repmat(A, r, c)
Syntax: repmat(A, [r c])
repmat(A, n) tiles A n-by-n times.
Example: repmat([1 2], 2, 2)   % [1 2 1 2; 1 2 1 2]
Differences from MATLAB: Two-dimensional arrays only, including cell and struct arrays. Container results are limited to 65536 slots.

## reshape
Change the shape of an array.
Syntax: reshape(A, r, c)
Syntax: reshape(A, [r c])
One size may be [] and is computed. Elements are taken in column order.
Example: reshape(1:6, 2, 3)   % [1 3 5; 2 4 6]
Differences from MATLAB: Two-dimensional results only; trailing ones are accepted.

## rethrow
Raise a caught error again.
Syntax: rethrow(err)
Takes one scalar struct with a character message and optional character identifier. Raises that message and identifier for an enclosing catch block. The stack field is not restored, and an empty message still raises an error.
Example: try; try; error('r:q','rq 3'); catch e; rethrow(e); end; catch outer; disp(outer.message); end   % rq 3
Differences from MATLAB: MatRust uses structs with an empty stack, not MException objects.

## return
Stop the running function or script.
Syntax: return
In a function it returns to the caller. In a script it ends the run.
Example: for k = 1:3, if k == 2, return, end, disp(k), end   % prints: 1, then stops
Differences from MATLAB: Not checked against MATLAB yet.

## rmfield
Remove fields from a struct.
Syntax: T = rmfield(S, name)
Syntax: T = rmfield(S, names)
Accepts a character name or a cell array of names. Removes those fields from every element, preserving the struct shape and remaining field order. S is unchanged unless the result is assigned back to S. A missing field raises an error.
Example: s=struct('a',1,'b','x'); t=rmfield(s,'a'); t.b   % 'x'
Differences from MATLAB: none known.

## rng
Seed the session's random number generator.
Syntax: rng(seed)
Syntax: rng(seed, 'twister')
Syntax: rng('default')
Syntax: rng('shuffle')
Controls the shared rand, randn and randi stream. Seed must be an integer from 0 to 2^32-1. 'default' uses seed 0; 'shuffle' derives a seed from the current time. Seed 0 uses the same initial state as seed 5489. This command returns no value.
Example: rng('default'); rand(1,2)   % [0.814724 0.905792]
Differences from MATLAB: Only twister is supported. Querying or restoring a state struct is not supported; randn sequences differ from MATLAB.

## round
Round to the nearest integer.
Syntax: round(X)
Halves round away from zero.
Example: round(2.5)   % 3
Differences from MATLAB: Real and complex double 2-D arrays. round(A,n) (n decimal digits) is supported for complex input only; 'significant' is not supported. Complex results with zero imaginary parts are real.

## save
Save workspace variables to a MAT file.
Syntax: save('data.mat')
Syntax: save('data.mat', 'A', 'B', '-v7.3')
Syntax: save data.mat A
Without names it saves all variables. The default file is matlab.mat. Versions: -v6, -v7 (default), -v7.3; -nocompression needs -v7.
Example: A=5; save('data.mat','A')   % A is saved
Differences from MATLAB: Statement only; no output assignment. No -append, ASCII, structure fields or wildcards.

## saveas
Save a figure in a chosen format.
Syntax: saveas(gcf, 'plot.svg')
Syntax: saveas(gcf, 'plot', 'svg')
Formats: fig, png, jpg, svg, pdf, bmp, tif, and 'epsc' when given explicitly. Call it as a statement.
Example: saveas(gcf, 'plot.svg')   % writes plot.svg
Differences from MATLAB: PNG, PDF and EPS export need the desktop build; SVG works everywhere. No monochrome or Level-2 EPS, EMF or M-file output.

## savefig
Save a figure to a file.
Syntax: savefig('plot.mrfig')
Syntax: savefig(n, 'plot.mrfig')
Saves the current figure, or figure n. Extensions are .fig, .mrfig (MatRust native) or .mfig.
Example: plot(1:3); savefig('plot.mrfig')   % figure saved
Differences from MATLAB: Statement only. No figure arrays, version or visibility options.

## sawtooth
Sawtooth wave.
Syntax: sawtooth(t)
Syntax: sawtooth(t, width)
Period is 2*pi. width is a scalar from 0 to 1 and sets where the peak occurs (default 1).
Example: sawtooth(pi/2)   % -0.5
Differences from MATLAB: Real double 2-D arrays only.

## scatter
Scatter plot.
Syntax: scatter(x, y)
Syntax: scatter(x, y, s)
Syntax: scatter(x, y, 'r')
s is a scalar marker area. A single color letter or name/value pairs such as 'Marker' can follow.
Example: scatter([1 2 3], [2 4 3])   % three markers
Differences from MATLAB: Real coordinates only. No per-point size or color arrays and no extra datasets.

## semilogx
Plot with a logarithmic x axis.
Syntax: semilogx(x, y)
Syntax: semilogx(Z)
The y axis stays linear. A single complex array plots real(Z) against imag(Z).
Example: semilogx([1 10 100], [1 2 3])   % x on log scale
Differences from MATLAB: Properties are trailing name/value pairs such as 'LineWidth', 2.

## semilogy
Plot with a logarithmic y axis.
Syntax: semilogy(x, y)
Syntax: semilogy(Z)
The x axis stays linear. A single complex array plots real(Z) against imag(Z).
Example: semilogy([1 2 3], [1 10 100])   % y on log scale
Differences from MATLAB: Properties are trailing name/value pairs such as 'LineWidth', 2.

## set
Change properties of a graphics object.
Syntax: set(h, 'PropertyName', value)
Syntax: set(h, 'Name1', value1, 'Name2', value2)
Up to 16 property and value pairs. Axes: XLim, YLim, XScale, YScale, XGrid, YGrid, NextPlot and others. Figure: Name, Theme, ThemeMode, Color. Lines: LineWidth, Visible.
Example: plot(1:3); set(gca, 'XLim', [0 5]); get(gca, 'XLim')   % [0 5]
Differences from MATLAB: Not checked against MATLAB yet. Unknown properties or values are rejected with an error. Call it as a statement.

## setdiff
Values in A that are not in B.
Syntax: C = setdiff(A, B)
Syntax: [C, ia] = setdiff(A, B, 'stable')
Result is sorted by default; 'stable' keeps the order of A. ia holds first indices in A.
Example: setdiff([1 2 3 4], [2 4])   % [1 3]
Differences from MATLAB: Numeric inputs support real/logical vectors and shaped empties with 'sorted' or 'stable'. Character/cellstr forms also support 'rows' and 'legacy'; 'rows' warns and is ignored for cell inputs.

## setfield
Set a field in a scalar struct.
Syntax: T = setfield(S, field, value)
Syntax: T = setfield(S, field1, field2, ..., value)
Returns an updated struct without changing S unless assigned back to S. Follows nested scalar fields and creates missing struct fields along the path. Field names must be valid character names. Paths and resulting containers are limited to 16 nesting levels.
Example: s=struct('a',1,'b','x'); t=setfield(s,'a',7); t.a   % 7
Differences from MATLAB: Cell index arguments and nonscalar structs are not supported.

## setxor
Values in exactly one of two sets.
Syntax: C = setxor(A, B)
Syntax: [C, ia, ib] = setxor(A, B, 'stable')
Result is sorted by default; 'stable' keeps input order.
Example: setxor([1 2 3], [3 4])   % [1 2 4]
Differences from MATLAB: Numeric inputs support real/logical vectors and shaped empties with 'sorted' or 'stable'. Character/cellstr forms also support 'rows' and 'legacy'; 'rows' warns and is ignored for cell inputs.

## sgtitle
Title for a whole figure.
Syntax: sgtitle('Title')
Syntax: sgtitle(fig, 'Title', 'Interpreter', 'none')
Sets one shared title above all axes of the figure. fig is an optional figure handle.
Example: sgtitle('Results')   % shared title
Differences from MATLAB: Only literal text with Interpreter none. No text handle output or other text properties.

## sign
Sign of real values or direction of complex values.
Syntax: sign(X)
Real elements give -1, 0 or 1; NaN stays NaN. A nonzero complex value gives X/abs(X), and complex zero gives zero. Logical inputs convert to double.
Example: sign([-2 0 3 NaN])   % [-1 0 1 NaN]
Differences from MATLAB: Only 2-D arrays are supported.

## sin
Sine of argument in radians.
Syntax: y = sin(x)
Elementwise sine; complex input is supported.
Example: sin(pi/2)   % 1
Example: sin(1+2i)   % 3.1658 + 1.9596i
Differences from MATLAB: logical input is not supported yet.

## sinc
Normalized sinc function.
Syntax: sinc(x)
Returns sin(pi*x)/(pi*x), with 1 at x = 0.
Example: sinc(0)   % 1
Differences from MATLAB: Real 2-D arrays only.

## single
Convert to single precision.
Syntax: single(X)
Each element takes 4 bytes. Arithmetic with double, logical or char values gives single, computed in single precision.
Example: x = single(pi)
Example: y = single(16777216) + 1   % stays 16777216
Differences from MATLAB: complex single is not supported yet; functions compute in double and round the result to single.

## size
Size of an array.
Syntax: size(A)
Syntax: size(A, dim)
Syntax: [r, c] = size(A)
size(A) returns [rows columns]. size(A, 1) is rows, size(A, 2) is columns, larger dim gives 1.
Example: [r, c] = size(ones(2, 3))   % r = 2, c = 3
Differences from MATLAB: Two-dimensional arrays only; no size(A, [d1 d2]) form.

## sort
Sort array elements.
Syntax: B = sort(A)
Syntax: B = sort(A, dim)
Syntax: B = sort(A, dim, 'descend')
Syntax: [B, I] = sort(A, ..., 'MissingPlacement', value)
Sorts each column of A in ascending order, or along dimension dim.
I holds the original positions, so B equals A(I) for a vector.
Equal values keep their original order. NaN values go last when ascending.
Example: [B, I] = sort([3 1 2])   % B = [1 2 3], I = [2 3 1]
Differences from MATLAB: Real, logical, complex, character and cellstr 2-D arrays are supported. Complex values order by abs, then angle in (-pi, pi], NaN last, stable in both directions; complex input accepts dim and 'ascend'/'descend' only. Cellstr input accepts no sorting options.

## sortrows
Sort rows of a matrix.
Syntax: B = sortrows(A)
Syntax: [B, I] = sortrows(A, cols, 'descend', 'MissingPlacement', 'last')
cols is a vector of signed column numbers; a negative number sorts that column in descending order. 'ascend' or 'descend' applies to all columns. NaN placement is auto, first or last.
Example: sortrows([3 1; 1 2; 2 0])   % [1 2; 2 0; 3 1]
Differences from MATLAB: Real, logical, character and cellstr 2-D arrays are supported. No tables or per-column direction arrays. Cellstr input accepts numeric column selectors, including negative columns, but no text options. Complex input sorts by abs, then angle, with numeric column selectors only.

## sosfilt
Filter with second-order sections.
Syntax: y = sosfilt(sos, x)
sos is an N-by-6 matrix, for example from butter_sos or cheby1_sos.
Example: y = sosfilt(butter_sos(2, 0.3), ones(1, 20))   % low-pass step response
Differences from MATLAB: No gain argument and no initial state; two inputs only.

## sosfreqz
Plot the magnitude response of SOS filters.
Syntax: sosfreqz(sos)
Syntax: sosfreqz(sos, n)
Plots dB magnitude at n points (default 512, at most 65536). It returns no values; use sosfreqzmag for numbers.
Example: sosfreqz(butter_sos(2, 0.3), 256)   % plots magnitude in dB
Differences from MATLAB: Plot only. No outputs, no sampling-rate argument.

## sosfreqzmag
Magnitude response of SOS filters.
Syntax: m = sosfreqzmag(sos)
Syntax: m = sosfreqzmag(sos, n)
Returns n linear magnitude values as a column (default 512).
Example: m = sosfreqzmag(butter_sos(2, 0.3), 128);   % 128-by-1 column
Differences from MATLAB: MATLAB has no sosfreqzmag.

## sosgrpdelay
Group delay of SOS filters.
Syntax: d = sosgrpdelay(sos)
Syntax: d = sosgrpdelay(sos, n)
Returns n values in samples as a column (default 512). Undefined points are NaN. A bare call plots it.
Example: d = sosgrpdelay(butter_sos(2, 0.3), 128);   % 128-by-1 column
Differences from MATLAB: MATLAB has no sosgrpdelay.

## sosimpz
Impulse response of SOS filters.
Syntax: h = sosimpz(sos, n)
Returns the first n samples as a column. n is required. A bare call plots it.
Example: h = sosimpz(butter_sos(2, 0.3), 32);   % 32-by-1 column
Differences from MATLAB: MATLAB has no sosimpz. n is required.

## sosphasez
Phase response of SOS filters.
Syntax: p = sosphasez(sos)
Syntax: p = sosphasez(sos, n)
Returns n phase values in radians as a column (default 512). A bare call plots it.
Example: p = sosphasez(butter_sos(2, 0.3), 128);   % 128-by-1 column
Differences from MATLAB: MATLAB has no sosphasez.

## sosstepz
Step response of SOS filters.
Syntax: s = sosstepz(sos, n)
Returns the first n samples as a column. n is required. A bare call plots it.
Example: s = sosstepz(butter_sos(2, 0.3), 32);   % 32-by-1 column
Differences from MATLAB: MATLAB has no sosstepz. n is required.

## sprintf
Format data into text.
Syntax: s = sprintf(format)
Syntax: s = sprintf(format, A, ...)
Returns a character row. format holds plain text and conversions such as %d, %i, %f, %e, %g, %s, %c, %x, %o and %%. Escapes such as \n and \t are converted. All values of all arguments are used in column order. If there are more values than conversions, the format is used again. Text arguments are used as one value for %s and as character codes for %d.
Example: sprintf('%d items, %.2f total', 3, 4.5)   % '3 items, 4.50 total'
Differences from MATLAB: Cell array arguments, file ids and string arrays are not supported yet.

## sqrt
Square root.
Syntax: sqrt(X)
Applies the square root to each element.
Negative real input and complex input give the principal complex root.
Example: sqrt(16)   % 4
Example: sqrt(-4)   % 0 + 2i
Differences from MATLAB: double, logical and char 2-D arrays only.

## square
Square wave.
Syntax: square(t)
Syntax: square(t, duty)
Period is 2*pi. duty is the percent of the period that is +1 (default 50).
Example: square(pi/2)   % 1
Differences from MATLAB: Real double 2-D arrays only.

## stairs
Stairstep plot.
Syntax: stairs(x, y)
Syntax: stairs(y, 'r--')
Draws steps through the points. A line spec or name/value pairs such as 'LineWidth', 2 can follow.
Example: stairs(1:4, [2 4 1 3])   % four steps
Differences from MATLAB: Real coordinates only.

## std
Standard deviation.
Syntax: std(A)
Syntax: std(A, w, dim)
Syntax: std(A, 0, 'all', 'omitnan')
w is 0 (divide by n-1, default) or 1 (divide by n). dim is a positive integer or 'all'. 'omitnan' or 'includenan' may follow.
Example: std([1 2 3 4])   % 1.2910
Differences from MATLAB: Real and complex double arrays, one output. Vector weights and vecdim are not supported. Complex input gives a real result from abs(x - mean).^2.

## stem
Stem plot.
Syntax: stem(x, y)
Syntax: stem(y, 'filled')
Draws a vertical line and marker for each point. 'filled' fills the markers; BaseValue sets the baseline.
Example: stem([1 3 2])   % three stems
Differences from MATLAB: Real coordinates only.

## stepz
Step response of a filter.
Syntax: s = stepz(b, a, n)
Returns the first n samples (1 to 65536) as a column. n is required. A bare call without assignment plots it.
Example: s = stepz([1 1], 1, 6)   % [1; 2; 2; 2; 2; 2]
Differences from MATLAB: n is required; one output.

## strcat
Join text horizontally.
Syntax: s = strcat(s1, s2, ...)
Syntax: c = strcat(c1, s2, ...)
For character arguments, trailing spaces of each argument are removed and the pieces are joined. If any argument is a cell array, the result is a cell array and cell elements keep their spaces. Cell arrays need the same size, or be 1-by-1.
Example: strcat('ab', 'cd', 'e')   % 'abcde'
Differences from MATLAB: Only character rows and cell arrays of them are accepted.

## strvcat
Stack text vertically, padding shorter rows with spaces.
Syntax: s = strvcat(s1, s2, ...)
Syntax: s = strvcat(C)
Character arrays and exact integer UTF-16 codes are stacked into a character matrix. A cell array of character rows is read in column-major order. Empty inputs are omitted.
Example: strvcat('a', 'bcd')   % rows 'a  ' and 'bcd'
Differences from MATLAB: Two-dimensional arrays only, at most 65536 UTF-16 units. Numeric inputs must be exact integers from 0 to 65535; rounding and out-of-range conversion are unsupported.

## strcmp
Compare text, case-sensitive.
Syntax: tf = strcmp(a, b)
Returns true if a and b are the same text. If one input is a cell array, the result is a logical array of the same size. Two cell arrays compare element by element. Other input types give false.
Example: strcmp({'a','b'}, 'a')   % [true false]
Differences from MATLAB: none known.

## strcmpi
Compare text, ignoring case.
Syntax: tf = strcmpi(a, b)
Same as strcmp, but upper and lower case letters are equal.
Example: strcmpi('ABC', 'abc')   % true
Differences from MATLAB: none known.

## strfind
Find text inside text.
Syntax: k = strfind(s, pattern)
Syntax: c = strfind(c, pattern)
Returns the start positions of pattern in s as a row, or [] if there is none. Matches may overlap. A cell array input gives a cell array of positions.
Example: strfind('abcabc', 'bc')   % [2 5]
Differences from MATLAB: The ForceCellOutput option is not supported yet.

## strjoin
Join the text pieces of a cell array.
Syntax: s = strjoin(c)
Syntax: s = strjoin(c, delimiter)
Puts delimiter between the pieces. The default delimiter is one space. Escapes such as \n in the delimiter are converted.
Example: strjoin({'a','b','c'}, '-')   % 'a-b-c'
Differences from MATLAB: A cell array of delimiters is not supported yet.

## strncmp
Compare the first n characters of text, case-sensitive.
Syntax: tf = strncmp(a, b, n)
Same as strcmp, but only the first n characters count. A text shorter than n is equal only to identical text. n = 0 gives true.
Example: strncmp('hello', 'help', 3)   % true
Differences from MATLAB: none known.

## strncmpi
Compare the first n characters of text, ignoring case.
Syntax: tf = strncmpi(a, b, n)
Same as strncmp, but upper and lower case letters are equal.
Example: strncmpi({'ABx','aby'}, 'abz', 2)   % [true true]
Differences from MATLAB: none known.

## strrep
Replace text.
Syntax: s = strrep(s, old, new)
Syntax: c = strrep(c, old, new)
Replaces every occurrence of old by new. A cell array input gives a cell array. An empty old leaves the text unchanged.
Example: strrep('hello world', 'o', '0')   % 'hell0 w0rld'
Differences from MATLAB: none known.

## strsplit
Split text at delimiters.
Syntax: c = strsplit(s)
Syntax: c = strsplit(s, delimiter)
Syntax: c = strsplit(s, delimiter, 'CollapseDelimiters', tf)
Returns a 1-by-n cell array of character rows. The default delimiter is any white space (space, \f, \n, \r, \t, \v). delimiter can be text or a cell array of text. Repeated delimiters count as one unless CollapseDelimiters is false. At most 65536 pieces.
Example: c = strsplit('a,b,c', ',')   % c = {'a', 'b', 'c'}
Differences from MATLAB: The DelimiterType option is not supported yet.

## strtrim
Remove spaces at both ends of text.
Syntax: s = strtrim(s)
Syntax: c = strtrim(c)
Removes leading and trailing white space (spaces, tabs, line breaks). A cell array of text gives a cell array.
Example: strtrim('  ab c  ')   % 'ab c'
Differences from MATLAB: Character matrices are not supported yet.

## struct
Create a structure with named fields.
Syntax: struct()
Syntax: struct(field, value, ...)
With no arguments, creates a scalar struct with no fields. Field names must be unique ASCII identifiers of at most 63 characters; up to 1024 fields are allowed. Non-cell values repeat across the struct array; nonscalar cell values set its shape and distribute their contents by element. All nonscalar cell shapes must match. A scalar cell contributes its content to every element; use a nested cell to keep a cell as a field value. Dot access and dynamic access such as S.(name) are supported.
Example: s=struct('a',{1,2,3}); s(2).a   % 2
Differences from MATLAB: Some struct display spacing differs. Only 2-D structs are supported; containers are limited to 65536 slots and 16 nesting levels. Object conversion forms are not supported.

## subplot
Select a grid cell for plotting.
Syntax: subplot(rows, cols, index)
Syntax: subplot('Position', [left bottom width height])
index may be a vector to span cells. Rows and columns are 1 to 16. Add 'replace' as the fourth argument to replace existing axes.
Example: subplot(2, 1, 1)   % top of two stacked plots
Differences from MATLAB: The three-digit form subplot(211) is not supported.

## sum
Sum of elements.
Syntax: sum(A)
Returns the sum of each column, or one value for a row vector. Logical input is allowed.
Example: sum([1 2 3])   % 6
Differences from MATLAB: No 'all', type or NaN options. The sum of [] is 0. Complex input sums the real and imaginary parts with the real kernel and gives a real result when every imaginary part is zero.

## svd
Singular values and singular vectors.
Syntax: s = svd(A)
Syntax: [U,S,V] = svd(A)
Syntax: [U,S,V] = svd(A, 'econ')
Syntax: [U,S,V] = svd(A, 0)
One output returns singular values in descending order as a column. Three outputs satisfy A approximately equal to U*S*V'. For m-by-n input and k=min(m,n), full output sizes are m-by-m, m-by-n and n-by-n. 'econ' gives m-by-k, k-by-k and n-by-k; option 0 uses economy only when m>n. Inputs must be finite and may be complex (S stays real); logical input converts to double.
Example: svd([1 2;3 4;5 6])   % [9.525518; 0.514301]
Differences from MATLAB: Singular vector signs (complex phases) and small singular values can differ; complex singular values can differ in the last digit, and a singular complex matrix can give an exact 0 where MATLAB gives about 1e-16. Two outputs are not supported.

## tan
Tangent of an angle in radians.
Syntax: tan(X)
Applies tangent to each element; complex input is supported.
Example: tan(0)   % 0
Differences from MATLAB: double 2-D arrays only.

## tic
Start a timer or return an independent timer identifier.
Syntax: tic
Syntax: id = tic
Syntax: id = tic()
Without an output, starts or replaces the session timer used by toc. With an output, returns an independent timer identifier and leaves the session timer unchanged. Pass that identifier to toc(id).
Example: id=tic; elapsed=toc(id); elapsed>=0   % true
Differences from MATLAB: Identifiers are positive double values in MatRust; do not depend on their exact value.

## tiledlayout
Create a tiled layout.
Syntax: tiledlayout(rows, cols)
Syntax: tiledlayout('flow')
'vertical' and 'horizontal' are also accepted. Select tiles afterward with nexttile.
Example: tiledlayout(2, 2)   % 2-by-2 grid of tiles
Differences from MATLAB: No layout handle output, spacing options or other arguments.

## title
Set the plot title.
Syntax: title('My plot')
Syntax: title('My plot', 'Interpreter', 'none', 'Color', [1 0 0])
The text must be quoted, up to 4096 bytes. Only Interpreter none and an RGB Color are accepted.
Example: title('Signal')   % title shown above the axes
Differences from MATLAB: No other text properties; no handle output.

## toc
Read elapsed time in seconds.
Syntax: toc
Syntax: elapsed = toc
Syntax: toc(id)
Syntax: elapsed = toc(id)
Without an id, reads the timer started by tic without an output; it errors if none was started. An id comes from id=tic. With an output, returns a nonnegative double scalar. Without an output, prints 'Elapsed time is ... seconds.' Reading a timer does not reset it.
Example: tic; elapsed=toc; elapsed>=0   % true
Differences from MATLAB: Measured times vary; error wording differs.

## trace
Sum of a square matrix's diagonal.
Syntax: trace(A)
Returns one scalar. Real, complex and logical square arrays are accepted; logical values convert to double. A nonsquare input raises an error.
Example: trace([1 2;3 4])   % 5
Differences from MATLAB: none known; a complex sum with zero imaginary part is real.

## transpose
Matrix transpose.
Syntax: transpose(A)
Same as A.'. Swaps rows and columns without conjugation for complex input.
Example: transpose([1 2 3])   % [1; 2; 3]
Differences from MATLAB: none known.

## trapz
Trapezoidal integral.
Syntax: trapz(Y)
Syntax: trapz(X, Y)
Syntax: trapz(Y, dim) or trapz(X, Y, dim)
X is a vector of sample points; without X the spacing is 1.
Example: trapz([1 2 3])   % 4
Differences from MATLAB: 2-D data; scalar spacing or a vector X. Complex Y works with a real vector X, where a scalar X is an error as in MATLAB; complex X is not supported.

## triang
Triangular window.
Syntax: triang(n)
Returns an n-by-1 column vector with no zero end points.
Example: triang(5)'   % [0.3333 0.6667 1 0.6667 0.3333]
Differences from MATLAB: No sampling option; double output only.

## tril
Lower triangular part.
Syntax: tril(A)
Syntax: tril(A, k)
Keeps entries on and below diagonal k (default 0) and zeros the rest.
Example: tril([1 2; 3 4])   % [1 0; 3 4]
Differences from MATLAB: Real 2-D arrays only.

## triu
Upper triangular part.
Syntax: triu(A)
Syntax: triu(A, k)
Keeps entries on and above diagonal k (default 0) and zeros the rest.
Example: triu([1 2; 3 4])   % [1 2; 0 4]
Differences from MATLAB: Real 2-D arrays only.

## true
Logical ones.
Syntax: true
Syntax: true(n)
Syntax: true(r, c) or true([r c])
true alone is a logical scalar; true(n) is n-by-n.
Example: true(1, 3)   % [true true true]
Differences from MATLAB: Two-dimensional output only.

## try
Run statements with optional error handling.
Syntax: try; statements; catch err; handler; end
Syntax: try; statements; catch; handler; end
Syntax: try; statements; end
On success, skips the catch block. A catchable runtime error skips the remaining try statements and runs the handler. Without a catch block, a catchable error is suppressed. Blocks may be nested; assignments completed before an error remain available. Parse errors are rejected before execution.
Example: try; error('stop'); catch; disp('caught'); end   % caught
Differences from MATLAB: The catch value is a struct with an empty stack. Cancellation and execution limits cannot be caught.

## typecast
Reinterpret the bytes of a numeric vector as another class.
Syntax: typecast(X, CLASSNAME)
X is a real integer, single or double vector; the byte count must be a multiple of the new element size. A row stays a row, a column stays a column.
Example: x = typecast(single(1), 'uint32')
Differences from MATLAB: none known.

## uint16
Convert to unsigned 16-bit integers.
Syntax: uint16(X)
Rounds half away from zero and saturates at intmin('uint16')..intmax('uint16'); NaN becomes 0. Each element takes 2 bytes.
Arithmetic with the same class or a scalar double stays uint16; other mixes are errors.
Example: x = uint16([2.5 -3.5 1000])
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## uint32
Convert to unsigned 32-bit integers.
Syntax: uint32(X)
Rounds half away from zero and saturates at intmin('uint32')..intmax('uint32'); NaN becomes 0. Each element takes 4 bytes.
Arithmetic with the same class or a scalar double stays uint32; other mixes are errors.
Example: x = uint32([2.5 -3.5 1000])
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## uint64
Convert to unsigned 64-bit integers.
Syntax: uint64(X)
Rounds half away from zero and saturates at intmin('uint64')..intmax('uint64'); NaN becomes 0. Each element takes 8 bytes.
Arithmetic with the same class or a scalar double stays uint64; other mixes are errors.
Example: x = uint64([2.5 -3.5 1000])
uint64 values stay exact 64-bit integers.
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## uint8
Convert to unsigned 8-bit integers.
Syntax: uint8(X)
Rounds half away from zero and saturates at intmin('uint8')..intmax('uint8'); NaN becomes 0. Each element takes 1 byte.
Arithmetic with the same class or a scalar double stays uint8; other mixes are errors.
Example: x = uint8([2.5 -3.5 1000])
Differences from MATLAB: complex input and the 'like' form are not supported yet.

## union
Combine two sets.
Syntax: C = union(A, B)
Syntax: [C, ia, ib] = union(A, B, 'stable')
Result is sorted by default; 'stable' keeps input order.
Example: union([1 2], [2 3])   % [1 2 3]
Differences from MATLAB: Numeric inputs support real/logical vectors and shaped empties with 'sorted' or 'stable'. Character/cellstr forms also support 'rows' and 'legacy'; 'rows' warns and is ignored for cell inputs.

## unique
Unique values.
Syntax: C = unique(A)
Syntax: [C, ia, ic] = unique(A, 'stable')
Sorted by default; 'stable' keeps first-occurrence order. ia gives first occurrences. Each NaN counts as distinct.
Example: unique([3 1 3 2])   % [1 2 3]
Differences from MATLAB: Real/logical 2-D arrays and character/cellstr inputs are supported. Text forms support 'rows', 'stable', 'sorted', 'first', 'last' and 'legacy'. For cellstr, 'rows' warns and is ignored. Numeric inputs retain the existing limited options. Complex input supports 'rows', 'stable', 'sorted', 'first' and 'last' (not 'legacy'), in the order of sort; NaNs stay distinct.

## upfirdn
Upsample, filter, then downsample.
Syntax: upfirdn(x, h)
Syntax: upfirdn(x, h, p, q)
h is the FIR filter. p is the upsampling factor and q the downsampling factor (default 1).
Example: upfirdn([1 2 3], [1 1])   % [1 3 5 3]
Differences from MATLAB: Real numeric 2-D arrays; filter values must be finite.

## upper
Convert text to upper case.
Syntax: s = upper(s)
Syntax: c = upper(c)
A cell array of text gives a cell array. Numeric input is returned unchanged.
Example: upper('abc')   % 'ABC'
Differences from MATLAB: Character matrices are not supported yet.

## upsample
Insert zeros between samples.
Syntax: upsample(x, n)
Syntax: upsample(x, n, phase)
Inserts n-1 zeros after each sample. phase is an integer from 0 to n-1.
Example: upsample([1 2 3], 2)   % [1 0 2 0 3 0]
Differences from MATLAB: Real numeric 2-D arrays only.

## var
Variance.
Syntax: var(A)
Syntax: var(A, w, dim)
Syntax: var(A, 0, 'all', 'omitnan')
w is 0 (divide by n-1, default) or 1 (divide by n). dim is a positive integer or 'all'. 'omitnan' or 'includenan' may follow.
Example: var([1 2 3 4])   % 1.6667
Differences from MATLAB: Real and complex double arrays, one output. Vector weights and vecdim are not supported. Complex input gives a real result from abs(x - mean).^2.

## while
Repeat statements while a condition is true.
Syntax: while condition, statements, end
The condition is tested before each pass, as for if. A loop runs at most 1000000 steps per run. Use break to leave early.
Example: n = 0; while n < 3, n = n + 1; end, n   % n = 3
Differences from MATLAB: Not checked against MATLAB yet. The total of 1000000 execution steps per run is a MatRust limit.

## whos
List workspace variables.
Syntax: whos
Prints name, size, bytes and class for each variable.
Example: whos   % one line per variable
Differences from MATLAB: No name filters and no structure output.

## writematrix
Write a numeric matrix to a text file.
Syntax: writematrix(A, 'out.csv')
Syntax: writematrix(A, 'out.csv', 'Delimiter', ';')
Delimiter is comma (default), semicolon or tab. Use a .csv, .txt or .dat name, or add 'FileType', 'text'. Call it as a statement.
Example: writematrix([1 2; 3 4], 'out.csv')   % writes two lines: 1,2 and 3,4
Differences from MATLAB: Statement only; no output assignment. Overwrites the file; no spreadsheets or append.

## xlabel
Set the x axis label.
Syntax: xlabel('Time (s)')
Syntax: xlabel('Time (s)', 'Interpreter', 'none', 'Color', [1 0 0])
The text must be quoted, up to 4096 bytes. Only Interpreter none and an RGB Color are accepted.
Example: xlabel('Time (s)')   % label below the x axis
Differences from MATLAB: No other text properties; no handle output.

## xlim
Set x axis limits.
Syntax: xlim([a b])
Syntax: xlim('auto')
Sets the visible x range, or returns it to automatic.
Example: xlim([0 5])   % x from 0 to 5
Differences from MATLAB: Numeric two-element limits or 'auto' only.

## xor
Element-wise exclusive OR.
Syntax: xor(A, B)
True where exactly one of A and B is nonzero.
Example: xor([1 0], [1 1])   % [false true]
Differences from MATLAB: Exactly two arguments.

## xscale
Set the x axis scale.
Syntax: xscale('linear')
Syntax: xscale('log')
Switches the x axis between linear and log scale.
Example: xscale('log')   % x axis uses log scale
Differences from MATLAB: Only 'linear' and 'log'.

## ylabel
Set the y axis label.
Syntax: ylabel('Amplitude')
Syntax: ylabel('Amplitude', 'Interpreter', 'none', 'Color', [1 0 0])
The text must be quoted, up to 4096 bytes. Only Interpreter none and an RGB Color are accepted.
Example: ylabel('Amplitude')   % label beside the y axis
Differences from MATLAB: No other text properties; no handle output.

## ylim
Set y axis limits.
Syntax: ylim([a b])
Syntax: ylim('auto')
Sets the visible y range, or returns it to automatic.
Example: ylim([-1 1])   % y from -1 to 1
Differences from MATLAB: Numeric two-element limits or 'auto' only.

## yscale
Set the y axis scale.
Syntax: yscale('linear')
Syntax: yscale('log')
Switches the y axis between linear and log scale.
Example: yscale('log')   % y axis uses log scale
Differences from MATLAB: Only 'linear' and 'log'.

## zeros
Create an array of all zeros.
Syntax: Z = zeros(n)
Syntax: Z = zeros(rows, cols)
Syntax: Z = zeros([rows cols])
zeros(n) is n-by-n. A size vector such as size(A) also works.
Negative sizes count as zero.
Syntax: zeros(..., 'single')
Syntax: zeros(..., 'like', single(1))
Example: zeros(2, 'single')
Example: zeros(2, 3)
Class names 'single' and int8 through uint64 are accepted, as are typed integer/single 'like' prototypes.
Differences from MATLAB: two-dimensional output only; zeros with no size is not supported.

## run
Run a script file.
Syntax: run(name)
Syntax: run('name.m')
Syntax: run('folder/name.m')
Runs the .mr or .m script name in your workspace, as typing name does.
When both name.mr and name.m exist, name.mr runs.
With a folder part (relative to the Current Folder, or absolute), the script
finds other files and saves and loads data in that folder while it runs;
afterwards the old folder is used again.
A function file runs with no inputs, as typing its name does.
Example: run('setup.m')
Differences from MATLAB: command syntax such as run setup is not supported
yet; use run('setup'). At most 16 other folders per run, and scripts and
functions share a call depth of 32 (MATLAB allows 500).

## switch
Choose a block of statements by value.
Syntax: switch expr; case value; statements; otherwise; statements; end
Compares expr with each case value in turn and runs the first block that matches; at most one block runs. Numbers match by value, character rows match exactly, and a cell array of values matches when any element does. A single character compared with a number compares by character code. Use otherwise for the block that runs when nothing matches.
Example: x = 2; switch x; case 1; disp('one'); case {2, 3}; disp('two or three'); otherwise; disp('other'); end   % two or three
Differences from MATLAB: Multi-character text never matches a number.

## case
Give one branch of a switch block.
Syntax: case value
Syntax: case {value1, value2}
Runs the statements up to the next case, otherwise or end when the switch expression matches value. A cell array of values matches when any element matches. Only valid inside switch.
Example: s = 'b'; switch s; case 'a'; disp(1); case {'b','c'}; disp(2); end   % 2
Differences from MATLAB: none known.

## otherwise
Give the default branch of a switch block.
Syntax: otherwise
Runs its statements when no case of the enclosing switch matched. Must come after every case. Only valid inside switch.
Example: switch 5; case 1; disp('one'); otherwise; disp('not one'); end   % not one
Differences from MATLAB: none known.

## feval
Call a function by handle or by name.
Syntax: [y1, ..., yN] = feval(f, x1, ..., xM)
Syntax: feval('name', x1, ..., xM)
f may be a function handle or a character row naming a built-in or user function. The remaining arguments are passed through, and the outputs are returned.
Example: feval(@max, [3 9 4])   % 9
Differences from MATLAB: none known.

## func2str
Return the text of a function handle.
Syntax: s = func2str(f)
For a named handle the result is the function name; for an anonymous function it is the full text starting with @.
Example: func2str(@(x) x.^2 + 1)   % '@(x)x.^2+1'
Differences from MATLAB: Anonymous function text is rebuilt from the parsed tokens, so spacing can differ from what was typed.

## str2func
Make a function handle from text.
Syntax: f = str2func('name')
Syntax: f = str2func('@(x) expression')
A name gives a handle to that function; text starting with @ gives an anonymous function. Variables in the text are not captured from the workspace.
Example: f = str2func('@(x) x + 1'); f(2)   % 3
Differences from MATLAB: none known.

## arrayfun
Apply a function to each element of an array.
Syntax: A = arrayfun(f, X)
Syntax: [A, B] = arrayfun(f, X, Y)
Syntax: C = arrayfun(f, X, 'UniformOutput', false)
Calls f once per element (all inputs must have the same size) and collects the results in an array of that size. Each call must return scalars of one class unless UniformOutput is false, which collects the results in cell arrays.
Example: arrayfun(@(x) x^2, [1 2 3])   % [1 4 9]
Differences from MATLAB: ErrorHandler is not supported.

## cellfun
Apply a function to each cell of a cell array.
Syntax: A = cellfun(f, C)
Syntax: A = cellfun('isempty', C)
Syntax: C2 = cellfun(f, C, 'UniformOutput', false)
Calls f once per cell (all cell inputs must have the same size) and collects scalar results in an array, or in cell arrays when UniformOutput is false. The legacy names 'isempty', 'length', 'isreal' and similar are accepted as text.
Example: cellfun(@length, {'ab', 'cde', ''})   % [2 3 0]
Differences from MATLAB: ErrorHandler is not supported.

## isequal
Test whether arrays are equal.
Syntax: tf = isequal(A, B)
Syntax: tf = isequal(A, B, C, ...)
True when every input has the same size and the same values. Numeric, logical and character inputs compare by value regardless of class. NaN is never equal to NaN.
Example: isequal([1 2 3], [1 2 3])   % 1
Differences from MATLAB: none known.

## lu
LU factorization with partial pivoting.
Syntax: [L, U, P] = lu(A)
Syntax: [L, U] = lu(A)
Syntax: Y = lu(A)
Syntax: [L, U, p] = lu(A, 'vector')
Syntax: [L, U, P] = lu(A, 'matrix')
P*A = L*U with L unit lower triangular. With two outputs L is the
permuted lower factor, so A = L*U. With one output Y holds U on and above
the diagonal and the multipliers of L below it. 'vector' returns the row
permutation as a row vector p, so A(p,:) = L*U. A may be rectangular.
Example: [L, U, P] = lu([1 2 3; 4 5 6; 7 8 10])   % U(1,:) = 7 8 10
Differences from MATLAB: sparse input and other options are not
supported. Complex double input pivots by |re|+|im| as LAPACK does.
MatRust also accepts logical input.

## qr
QR factorization by Householder reflections.
Syntax: [Q, R] = qr(A)
Syntax: [Q, R] = qr(A, 0)
Syntax: [Q, R, E] = qr(A)
Syntax: [Q, R, p] = qr(A, 0)
Syntax: [Q, R, p] = qr(A, 'vector')
Syntax: [Q, R, E] = qr(A, 'matrix')
Syntax: H = qr(A)
Syntax: H = qr(A, 0)
A = Q*R with orthogonal Q. qr(A, 0) is the economy size when A has more
rows than columns. With three outputs columns are pivoted by their
remaining norm: A*E = Q*R, or A(:,p) = Q*R with the economy or 'vector' form.
With one output, qr(A) and qr(A, 0) both return the packed m-by-n factorization:
R on and above the diagonal and Householder vectors below it.
Example: [Q, R] = qr([1 2; 3 4])   % R = [-3.1623 -4.4272; 0 -0.6325]
Differences from MATLAB: sparse input is not supported. Complex input uses
the same unblocked Householder steps as LAPACK (column pivoting by remaining
norm for three outputs); results whose imaginary parts are all zero are real.

## chol
Cholesky factorization of a symmetric positive definite matrix.
Syntax: R = chol(A)
Syntax: R = chol(A, 'lower')
Syntax: [R, p] = chol(A)
R'*R = A with R upper triangular, reading only the upper triangle of A.
'lower' returns L with L*L' = A from the lower triangle. With a second
output a failure does not raise an error: p is 0 on success, otherwise
the order of the failing leading minor and R is the factor of the
(p-1)-by-(p-1) leading block.
Example: R = chol([4 2; 2 3])   % R = [2 1; 0 1.4142]
Differences from MATLAB: the 'vector' form and sparse input are not
supported. Complex input reads only the upper triangle (or the lower one for
'lower') and needs a real diagonal; a factor whose imaginary parts are all
zero is real.

## pinv
Moore-Penrose pseudoinverse.
Syntax: X = pinv(A)
Syntax: X = pinv(A, tol)
Computed from the singular value decomposition; singular values at or
below tol are treated as zero. The default tol is max(size(A))*eps(norm(A)).
Example: pinv([1 2; 2 4])   % [0.04 0.08; 0.08 0.16]
Differences from MATLAB: complex input is supported; results can differ from
MATLAB in the last digit. NaN or Inf in A is an error, as in MATLAB.

## null
Orthonormal basis of the null space.
Syntax: Z = null(A)
Columns of Z span {x : A*x = 0}. The basis comes from the singular value
decomposition; the number of columns is n minus the numerical rank.
The sign and rotation of the basis vectors can differ from MATLAB.
Example: null([1 2; 2 4])   % one column, proportional to [-2; 1]
Differences from MATLAB: complex input is supported; the phase of each complex
basis vector can differ. null(A,'r') is not supported.

## orth
Orthonormal basis of the range.
Syntax: Q = orth(A)
Columns of Q span the column space of A; the number of columns is the
numerical rank. The sign of each column can differ from MATLAB.
Example: orth([1 2; 2 4])   % one column, proportional to [1; 2]/sqrt(5)
Differences from MATLAB: complex input is supported; the phase of each complex
column can differ.

## expm
Matrix exponential.
Syntax: Y = expm(A)
Pade approximation with scaling and squaring. A must be square.
This is not the elementwise exp.
Example: expm([1 2; 3 4])   % [51.969 74.737; 112.105 164.074]
Differences from MATLAB: complex input is supported (results can
differ in the last digit). A non-square complex A gives 'Expected input to be a square matrix.'
MatRust also accepts logical input.
MatRust does not balance A first: badly scaled matrices can lose accuracy.

## logm
Matrix logarithm.
Syntax: L = logm(A)
Principal logarithm by Schur decomposition and inverse scaling and
squaring. The result is complex when A has negative real eigenvalues.
Example: logm([1 2; 3 4])   % complex 2-by-2
Differences from MATLAB: real and logical input only. A singular matrix
raises an error here.

## sqrtm
Matrix square root.
Syntax: X = sqrtm(A)
Principal square root by Schur decomposition, X*X = A. The result is
complex when A has negative real eigenvalues.
Example: sqrtm([4 1; 0 9])   % [2 0.2; 0 3]
Differences from MATLAB: real and logical input only.

## mldivide
Solve a linear system, A\B.
Syntax: X = mldivide(A, B)
Same as the operator A\B.
Example: mldivide([2 0; 0 4], [2; 4])   % [1; 1]
Differences from MATLAB: square A only, as for the operator; a singular A
raises an error. Complex input: a scalar A divides B elementwise; a square A is solved by LU (triangular A by substitution) with MATLAB's singular and RCOND warnings; rectangular complex systems are not implemented yet.

raises an error. With complex input, A may be a scalar (elementwise division of B by A) or a non-square matrix (least squares by column-pivoted QR with the rank-deficiency warning); square complex solves are not implemented yet.

## mrdivide
Right division, A/B.
Syntax: X = mrdivide(A, B)
Same as the operator A/B.
Example: mrdivide([2 4], [2 0; 0 4])   % [1 1]
Differences from MATLAB: square B only, as for the operator. Complex input: a scalar B divides A elementwise; a square B is solved as (B.'\A.').' with MATLAB's warnings; rectangular complex systems are not implemented yet.

Differences from MATLAB: square B only, as for the operator. With complex input, B may be a scalar (elementwise division of A by B) or a non-square matrix (least squares, (B.'\A.').'); square complex solves are not implemented yet.

## cond
Condition number for inversion.
Syntax: c = cond(A)
Syntax: c = cond(A, p)
p is 2 (default), 1, Inf or 'fro'. The 2-norm form uses singular values
and accepts rectangular A; the others need a square A. A singular A gives Inf.
Example: cond([1 2 3; 4 5 6; 7 8 10])   % 88.448
Differences from MATLAB: complex input is supported; results can differ from MATLAB in
the last digit, and a singular complex matrix can give Inf where MATLAB gives a
large finite value.

## norm
Vector and matrix norms.
Syntax: n = norm(A)
Syntax: n = norm(A, p)
Vectors: p is any real number, Inf, -Inf or 'fro'; the default is 2. Other p
give sum(abs(v).^p)^(1/p), so norm([3 4], -1) is 12/7 and norm([3 4], 0) is Inf.
Matrices: p is 1 (largest column sum), 2 (largest singular value, the
default), Inf (largest row sum) or 'fro'. norm([]) is 0.
Example: norm([3 4])   % 5
Differences from MATLAB: complex input is supported (|z| for the vector and matrix 1, Inf
and p norms, singular values for the matrix 2-norm). MatRust also accepts logical input;
'inf' is also accepted as text.

## fopen
Open a file and return a file identifier.
Syntax: fid = fopen(filename)
Syntax: fid = fopen(filename, permission)
Syntax: [fid, msg] = fopen(filename, permission)
Syntax: fid = fopen(filename, permission, machinefmt, encoding)
Syntax: ids = fopen('all')
Syntax: [name, permission] = fopen(fid)
permission is 'r' (default), 'w', 'a', 'r+', 'w+' or 'a+', optionally with b or t.
With t, writes translate LF to CR LF and reads translate CR LF to LF on all
platforms, matching Windows MATLAB. Without t, bytes are unchanged. File positions
remain byte offsets in the physical file, including CR bytes.
fid is 3 for the first open file, then 4, 5, ... and reuses the lowest free number.
When the file cannot be opened fid is -1 and msg explains why.
machinefmt 'n', 'l', 'a', 'ieee-le' or 'ieee-le.l64' (little endian, the default)
or 'b', 's', 'ieee-be' or 'ieee-be.l64' sets the byte order of later fread and
fwrite calls on that file; any other machinefmt is "Invalid machine format.".
encoding is 'UTF-8' (default) or 'ISO-8859-1' ('latin1'), one byte per character.
Relative names are taken from the current folder. At most 64 files are open at once.
Example: [fid, msg] = fopen('nothing.txt')   % fid = -1, msg = 'No such file or directory'
Differences from MATLAB: files over 8 MiB, devices and other special files,
network paths and URLs are refused; encodings other than UTF-8 and ISO-8859-1 are
not supported yet; fopen(fid) returns 'native' (or 'ieee-be') as machine format
and 'UTF-8' as default encoding, where MATLAB reports the system encoding
(for example 'US-ASCII').

## fclose
Close one open file or all of them.
Syntax: status = fclose(fid)
Syntax: status = fclose('all')
Returns 0. A file id that is not open is an error. Open files are also closed when
the session ends.
Example: fid = fopen('a.txt', 'w'); fclose(fid)   % 0
Differences from MATLAB: clear all is not supported yet, so use fclose('all').

## fgetl
Read one line of a file without the line break.
Syntax: tline = fgetl(fid)
Syntax: tline = fgetl(fid, nchar)
Syntax: [tline, ltout] = fgetl(fid)
Returns -1 when there is nothing left to read, and an empty character array for
a file opened only for writing. An empty line gives an empty character array.
A line ends at LF, CR LF or a lone CR; the terminator is removed and returned
as ltout.
Example: fid = fopen('data.txt'); l = fgetl(fid)   % '1,3'
Differences from MATLAB: bytes that are not valid UTF-8 are read one character
per byte.

## fgets
Read one line of a file and keep the line break.
Syntax: tline = fgets(fid)
Syntax: tline = fgets(fid, nchar)
Syntax: [tline, ltout] = fgets(fid)
Returns -1 when there is nothing left to read. Lines end as for fgetl; ltout is
the terminator (LF, CR LF or CR).
Example: fid = fopen('data.txt'); fgetl(fid); l = fgets(fid)   % '2,4' and a line break
Differences from MATLAB: same as fgetl.

## feof
Test for the end of a file.
Syntax: tf = feof(fid)
The result is true once a read has reached the end of the file; fseek (even
one that fails) and frewind clear it. fgetl and fgets report the end as soon as they read the last line.
Example: fid = fopen('data.txt'); fgetl(fid); fgetl(fid); feof(fid)   % true
Differences from MATLAB: none known.

## ftell
Current position in a file, in bytes from the start.
Syntax: position = ftell(fid)
Example: fid = fopen('b.bin'); fseek(fid, 2, 'bof'); ftell(fid)   % 2
Differences from MATLAB: none known.

## fseek
Move to a position in a file.
Syntax: status = fseek(fid, offset, origin)
origin is 'bof' (or -1), 'cof' (or 0) or 'eof' (or 1). Returns 0, or -1 when the
new position would be before the start or after the end; the position then stays.
Example: fseek(fid, -3, 'eof')   % 0, three bytes before the end

## frewind
Go back to the start of a file.
Syntax: frewind(fid)
Also clears the end-of-file flag.
Example: frewind(fid); ftell(fid)   % 0
Differences from MATLAB: as in MATLAB, frewind has no output; x = frewind(fid)
fails with "This command does not return a value" instead of "Too many output
arguments.".

## fwrite
Write binary data to a file.
Syntax: count = fwrite(fid, A)
Syntax: count = fwrite(fid, A, precision)
Syntax: count = fwrite(fid, A, precision, 0, machinefmt)
precision is 'uint8' (default), 'int8', 'uint16', 'int16', 'uint32', 'int32',
'uint64', 'int64', 'single', 'double' or 'char'. Integers round to the nearest
value and saturate. machinefmt is 'l' or 'b'; without it the byte order given
to fopen is used (little endian by default). Characters are written
as their character codes. Returns the number of elements written; a file
opened for reading only writes nothing and returns 0.
Example: fid = fopen('b.bin', 'w'); fwrite(fid, [1 2 3 255])   % 4
Differences from MATLAB: a skip value other than 0 and the N*type form are not supported.

## fread
Read binary data from a file.
Syntax: A = fread(fid)
Syntax: A = fread(fid, sizeA)
Syntax: A = fread(fid, sizeA, precision)
Syntax: [A, count] = fread(fid, sizeA, precision, 0, machinefmt)
sizeA is Inf (default), N or [m n]; with [m n] the last column is padded with zeros.
precision is as for fwrite; 'uint8=>char' and '*char' return characters.
All other output is double.
Example: [a, n] = fread(fid, [2 2])   % a = [1 3; 2 255], n = 4
Differences from MATLAB: output is double or char only, so '*uint8' and
'uint8=>uint8' return double; a skip value other than 0 is not supported.

## fscanf
Read formatted data from a file.
Syntax: A = fscanf(fid, format)
Syntax: A = fscanf(fid, format, sizeA)
Syntax: [A, count] = fscanf(fid, format, sizeA)
format may use %d %i %u %f %e %g %s %c, widths and %* to skip. The format is reused
until the data ends or does not match; a zero width such as %0d reads nothing.
%c also reads white space and line breaks. The result is a column of numbers; with
only %s and %c it is a character row. sizeA is Inf, N or [m n].
Example: A = fscanf(fid, '%d', [3 Inf])   % [1 4; 2 5; 3 6] for the file '1 2 3' and '4 5 6'
Differences from MATLAB: other conversions such as %x and %[ are not supported.

## textscan
Read delimited text into a cell array.
Syntax: C = textscan(fid, formatSpec)
Syntax: C = textscan(str, formatSpec)
Syntax: C = textscan(fid, formatSpec, N, 'Delimiter', d, 'HeaderLines', n, 'CollectOutput', tf)
formatSpec uses %f, %d, %n (numbers) and %s (text), with %* to skip a field.
Numbers give a column vector and text a cell column; an empty number field is NaN.
Line breaks end a record; missing fields on that line are NaN for numbers and
empty character arrays for text.
A field that does not convert ends the scan, and so does the end of the data in
the middle of a record; the fields read before are kept, so the columns can
differ in length. At the end of the data the file position is at the end and
feof is true, so a while ~feof(fid) loop ends.
Example: C = textscan('1 2 3 4', '%f %f'); C{2}   % [2; 4]
Differences from MATLAB: %d gives double, not int32; other conversions and options
are not supported; Delimiter takes a character vector, each character a delimiter.

## dlmread
Read a numeric file with delimiters.
Syntax: M = dlmread(filename)
Syntax: M = dlmread(filename, delimiter)
Syntax: M = dlmread(filename, delimiter, r, c)
Syntax: M = dlmread(filename, delimiter, [r1 c1 r2 c2])
The delimiter is found from the first line when omitted (comma, tab, semicolon,
or spaces). r and c count rows and columns to skip, starting from 0. Empty
fields and missing values at the end of a row read as 0; non-numeric fields
raise a mismatch error with the row and field number. A row offset at or past
the end of the data is the error "Empty format string is not supported at the
end of a file.".
Example: M = dlmread('m.txt', ',', 1, 0)   % the second row of m.txt
Differences from MATLAB: spreadsheet ranges such as 'A1..B2' and complex numbers are not supported.

## dlmwrite
Write a matrix to a delimited file.
Syntax: dlmwrite(filename, M)
Syntax: dlmwrite(filename, M, delimiter, r, c)
Syntax: dlmwrite(filename, M, '-append', 'precision', p, 'newline', nl)
The default delimiter is a comma and the default precision is %.5g. precision is
a number of significant digits or a format such as '%8.3f'. r and c add empty
rows and columns in front. newline is 'pc' or 'unix'. The file is only
written once all arguments are checked, so a rejected call leaves it unchanged.
Example: dlmwrite('m.txt', [1 2.5; 3 pi])   % file has "1,2.5" and "3,3.1416"
Differences from MATLAB: the real part of complex data is written; output over
8 MiB is refused; an invalid precision format such as '%z' is an error, where
MATLAB writes an empty line.

## csvread
Read a comma-separated numeric file.
Syntax: M = csvread(filename)
Syntax: M = csvread(filename, r, c)
Syntax: M = csvread(filename, r, c, [r1 c1 r2 c2])
Same as dlmread with a comma delimiter; r and c count from 0. With a range, r
and c must equal its first two values; an empty range is the error "Index
exceeds matrix dimensions.".
Example: csvread('c.csv')   % [1 2; 3 4]
Differences from MATLAB: when r and c differ from the range, MATLAB warns and
returns a zero-padded matrix; MatRust raises an error.

## csvwrite
Write a matrix to a comma-separated file.
Syntax: csvwrite(filename, M)
Syntax: csvwrite(filename, M, r, c)
Same as dlmwrite with a comma delimiter. r and c add empty rows and columns in front.
Example: csvwrite('c.csv', [1 2; 3 4])
Differences from MATLAB: output over 8 MiB is refused.

## type
Print a file in the command window.
Syntax: type('filename')
A blank line comes first, then the text of the file. When the name has no
extension and no such file exists, filename.m is shown.
Example: type('m.txt')
Differences from MATLAB: only the function form works, not the command form
type m.txt; files over 8 MiB are refused.

## exist
Check for a variable, file or folder.
Syntax: n = exist(name)
Syntax: n = exist(name, 'var' | 'file' | 'dir' | 'builtin')
Returns 1 for a variable, 2 for a file, 7 for a folder, 5 for a built-in function
and 0 when there is none. A name is looked up as variable, then file, then built-in.
Example: x = 5; exist('x')   % 1
Differences from MATLAB: every function MatRust provides reports 5, including
those that are M-files in MATLAB (MATLAB gives 2 for exist('mean')).

## format
Set how numbers are displayed.
Syntax: format
Syntax: format short
Syntax: format long
Syntax: format compact
Syntax: format loose
Changes how variables and ans are shown in the Command Window and by disp. short (the default) shows 4 digits after the decimal point, long shows 15 for doubles. compact drops the blank lines around displayed values; loose puts them back. format with no option resets to short and loose. Large or small values switch to a common scale factor or to e-notation as MATLAB does.
Example: format long; x = pi   % x = 3.141592653589793
Differences from MATLAB: Other options such as shortE, longG, bank, hex and rat are not supported yet. format long uses 7 decimal places for single values.

## conv
Convolution of two vectors, or polynomial multiplication.
Syntax: w = conv(u, v)
Syntax: w = conv(u, v, shape)
shape is 'full' (default), 'same' (central part, same size as u) or 'valid'
(only the part computed without zero padding).
With 'full' the result has the orientation of the longer input (of v when
both have the same length); with 'same' and 'valid' that of u.
Example: conv([1 2 3], [1 1])   % [1 3 5 3]
Differences from MATLAB: real, logical and complex double vectors only. The
product is computed directly, with no FFT shortcut. Complex results with zero
imaginary parts are real, as in MATLAB.

## deconv
Polynomial division with remainder.
Syntax: q = deconv(v, u)
Syntax: [q, r] = deconv(v, u)
q is the quotient and r the remainder, with v = conv(u, q) + r. q is a
column when v is a column, else a row; r has the shape of v. The first
coefficient of u must be nonzero. If u is longer than v then q = 0 and r = v.
Example: [q, r] = deconv([1 5 7], [1 2])   % q = [1 3], r = [0 0 1]
Differences from MATLAB: real and complex vectors use long division; the
last digits of q and r can differ from MATLAB's.

## roots
Roots of a polynomial given by its coefficients.
Syntax: r = roots(p)
Leading zeros are dropped and trailing zeros give roots at 0. The other
roots are the eigenvalues of the companion matrix. r is a column and is
complex only when some root has a nonzero imaginary part.
Example: roots([1 -3 2])   % [2; 1]
Differences from MATLAB: degree up to 1000. The order of roots with the same
size can differ from MATLAB. Complex coefficients use the engine's eig of the
companion matrix; the last digit and the order can differ from MATLAB's MKL.

## poly
Polynomial coefficients from roots, or the characteristic polynomial of a matrix.
Syntax: p = poly(r)
Syntax: p = poly(A)
For a vector r, p has the roots r. For a square matrix A, p is det(s*I - A)
computed from its eigenvalues. The result is real when the roots come in
complex conjugate pairs.
Example: poly([1 2 3])   % [1 -6 11 -6]
Differences from MATLAB: complex vectors and square matrices are supported; eigenvalues of a complex matrix come from the engine's eig.

## polyder
Derivative of a polynomial, or of a product of two polynomials.
Syntax: q = polyder(p)
Syntax: q = polyder(a, b)
Syntax: [q, d] = polyder(b, a)
Leading zeros are dropped and the result is a row. With two outputs, q/d is
the derivative of the quotient b/a, and d = conv(a, a).
Example: [q, d] = polyder([1 2], [1 3])   % q = 1, d = [1 6 9]
Differences from MATLAB: none known; complex input is supported.

## polyint
Integral of a polynomial.
Syntax: q = polyint(p)
Syntax: q = polyint(p, k)
k is the constant of integration (default 0); a row k is appended as is.
p must be a row or a scalar. The result is a row.
Example: polyint([3 2 1])   % [1 1 1 0]
Differences from MATLAB: none known.

## conv2
Two-dimensional convolution.
Syntax: C = conv2(A, B)
Syntax: C = conv2(A, B, shape)
Syntax: C = conv2(hcol, hrow, A)
Syntax: C = conv2(hcol, hrow, A, shape)
shape is 'full' (default), 'same' or 'valid'. The hcol, hrow form convolves
each column with hcol and each row with hrow.
Example: conv2([1 2;3 4], [1 1;1 1])   % [1 3 2; 4 10 6; 3 7 4]
Differences from MATLAB: the hcol, hrow, A form takes real data only.

## interp1
One-dimensional interpolation.
Syntax: vq = interp1(x, v, xq)
Syntax: vq = interp1(v, xq)
Syntax: vq = interp1(x, v, xq, method)
Syntax: vq = interp1(x, v, xq, method, extrapolation)
method is 'linear' (default), 'nearest', 'next', 'previous', 'pchip',
'cubic' (same as 'pchip'), 'spline' (not-a-knot) or 'v5cubic' (MATLAB 5
cubic convolution; 'spline' when x is not equally spaced). extrapolation is
'extrap' or a scalar fill value; outside the sample range the default is NaN,
except 'pchip', 'cubic' and 'spline', which extrapolate; 'v5cubic' never
extrapolates. If v is a matrix,
each column is interpolated and xq must be a vector.
Example: interp1([1 2 3], [4 5 7], [0 4], 'linear', 'extrap')   % [3 9]
Differences from MATLAB: x and xq must be real; complex v (and a complex fill
value) is interpolated part by part, as in MATLAB. With
'extrap', 'next' and 'previous' give NaN on the side that has no neighbour.

## interp2
Two-dimensional interpolation on a regular grid.
Syntax: vq = interp2(V, xq, yq)
Syntax: vq = interp2(X, Y, V, xq, yq)
Syntax: vq = interp2(..., method)
Syntax: vq = interp2(..., method, extrapval)
method is 'linear' (default) or 'nearest'. X and Y are grid vectors or
meshgrid matrices. If xq is a row and yq a column (or the reverse) the result
is the grid of all pairs; otherwise xq and yq are paired. Points outside the
grid give NaN, or extrapval.
Example: interp2([1 2 3;4 5 6;7 8 9], 1.5, 1.5)   % 3
Differences from MATLAB: 'cubic', 'spline' and 'makima' are not supported yet.

## spline
Cubic spline interpolation (not-a-knot end conditions).
Syntax: yy = spline(x, y, xx)
x and y are vectors of the same length. Values outside the data range are
extrapolated by the end polynomial. Two points give a line, three a parabola.
Example: spline([0 1 2 3], [0 1 8 27], [0.5 1.5])   % [0.125 3.375]
Differences from MATLAB: the forms spline(x, y) (piecewise polynomial
structure), matrix y and clamped end slopes are not supported yet.

## pchip
Shape-preserving piecewise cubic Hermite interpolation.
Syntax: yy = pchip(x, y, xx)
x and y are vectors of the same length. Values outside the data range are
extrapolated by the end polynomial.
Example: pchip([0 1 2 3], [0 1 1 5], [0.5 1.5 2.5])   % [0.6875 1 2.25]
Differences from MATLAB: the forms pchip(x, y) (piecewise polynomial
structure) and matrix y are not supported yet.

## fzero
Root of a scalar function.
Syntax: x = fzero(fun, x0)
Syntax: [x, fval, exitflag] = fzero(fun, x0)
x0 is a starting point or an interval [a b] whose end values differ in sign.
From a starting point x, fzero tries x - dx and x + dx (dx = x/50, or 1/50
when x is 0, times sqrt(2) each step) until the sign changes, then uses
Brent's method with tolerance eps.
If that search meets a NaN, Inf or complex value, or runs off to Inf, fzero
prints why and returns NaN with exitflag -3, -4 or -6. exitflag is -5 when
|f(x)| grew while the interval shrank (a pole, such as tan near pi/2).
Example: fzero('cos', 1)   % 1.5708
Differences from MATLAB: fun is the name of a function as text, for example
fzero('myfun', 1); function handles are not supported yet. Options are
ignored with a note, and the fourth output (output structure) is not supported.

## fminsearch
Minimum of a function of several variables (Nelder-Mead simplex).
Syntax: x = fminsearch(fun, x0)
Syntax: [x, fval, exitflag] = fminsearch(fun, x0)
x has the shape of x0. TolX and TolFun are 1e-4, and at most 200*numel(x0)
iterations and function evaluations are used. exitflag is 1 when converged
and 0 when the limit was reached.
Example: [x, fv] = fminsearch('cos', 3)   % x = 3.141577, fv = -1.000000
Differences from MATLAB: fun is the name of a function as text; function
handles are not supported yet. Options are ignored with a note.

## fminbnd
Minimum of a scalar function on an interval.
Syntax: x = fminbnd(fun, x1, x2)
Syntax: [x, fval, exitflag] = fminbnd(fun, x1, x2)
Golden-section search with parabolic interpolation, TolX 1e-4, at most 500
iterations.
Example: fminbnd('cos', 3, 4)   % 3.1416
Differences from MATLAB: fun is the name of a function as text; function
handles are not supported yet. Options are ignored with a note.

## fft2
Two-dimensional discrete Fourier transform.
Syntax: Y = fft2(X)
Syntax: Y = fft2(X, m, n)
Transforms down the columns, then along the rows. X is cropped or padded
with zeros to m-by-n first when given. Y is real when every imaginary part
is zero, as in MATLAB.
Example: fft2([1 2;3 4])   % [10 -2; -4 0]
Differences from MATLAB: real, logical and complex 2-D arrays only.

## ifft2
Inverse two-dimensional discrete Fourier transform.
Syntax: X = ifft2(Y)
Syntax: X = ifft2(Y, m, n)
The result is real when the transformed array (Y cropped or padded to
m-by-n when given) is conjugate symmetric.
Example: ifft2([10 -2;-4 0])   % [1 2; 3 4]
Differences from MATLAB: real, logical and complex 2-D arrays only.

## fftshift
Move the zero-frequency component to the center.
Syntax: Y = fftshift(X)
Syntax: Y = fftshift(X, dim)
Each shifted dimension of length n is rotated by floor(n/2) positions.
Example: fftshift([1 2 3 4 5])   % [4 5 1 2 3]
Differences from MATLAB: real, logical and complex 2-D arrays only.

## ifftshift
Undo fftshift.
Syntax: Y = ifftshift(X)
Syntax: Y = ifftshift(X, dim)
Each shifted dimension of length n is rotated by ceil(n/2) positions.
Example: ifftshift([1 2 3 4 5])   % [3 4 5 1 2]
Differences from MATLAB: real, logical and complex 2-D arrays only.

## mode
Most frequent value.
Syntax: M = mode(X)
Syntax: M = mode(X, dim)
Syntax: [M, F] = mode(X)
F counts how often M occurs. Ties give the smallest value. NaN values are
never counted as repeats. The empty input gives NaN.
Logical and char input give a logical or char M.
Example: [m, f] = mode([1 2 2 3 3])   % m = 2, f = 2
Differences from MATLAB: the third output (cell of all tied values) is not
supported yet. Complex input orders by abs, then angle, and returns the first most frequent value in that order.

## histc
Count values in bins given by edges.
Syntax: n = histc(x, edges)
Syntax: n = histc(x, edges, dim)
Syntax: [n, bin] = histc(x, edges)
n(k) counts x with edges(k) <= x < edges(k+1); n(end) counts x equal to
edges(end). bin gives the bin number of each element (0 if none). A vector
x gives a vector n of the same orientation. dim is 1 or 2.
Example: n = histc([1 2 2 3 5], [1 2 3 4 5])   % [1 2 1 0 1]
Differences from MATLAB: real 2-D input only; dim above 2 is an error.

## histcounts
Count values in bins.
Syntax: N = histcounts(X)
Syntax: N = histcounts(X, nbins)
Syntax: N = histcounts(X, edges)
Syntax: [N, edges] = histcounts(X, ...)
Bins are [edges(k), edges(k+1)) and the last bin is closed. NaN values are
ignored. Bins are equal width at "nice" positions, as in MATLAB: integer data
spanning at most 50 gets one bin per integer, other data Scott's rule. N is a
row.
Example: [N, edges] = histcounts([1 2 2 3 5 7], 3)   % N = [3 2 1], edges = [0 3 6 9]
Differences from MATLAB: name-value options and normalization are not
supported yet.

## gcd
Greatest common divisor.
Syntax: g = gcd(a, b)
Syntax: [g, c, d] = gcd(a, b)
Elementwise on integer-valued a and b (one may be a scalar); g is never
negative.
With three outputs, g = a.*c + b.*d.
Example: gcd(12, 18)   % 6
Differences from MATLAB: Real double and logical input only; both inputs must
have equal size or one must be a scalar; no integer classes.

## lcm
Least common multiple.
Syntax: l = lcm(a, b)
Elementwise on positive integer-valued a and b (one may be a scalar).
Zero or negative inputs are errors.
Example: lcm(4, 6)   % 12
Differences from MATLAB: Real double and logical input only; exactly two
inputs.

## factorial
Factorial of nonnegative integers.
Syntax: y = factorial(n)
Elementwise n!. Values for n above 170 are Inf.
Example: factorial(5)   % 120
Differences from MATLAB: Real double and logical input only.

## nchoosek
Binomial coefficient or all combinations.
Syntax: c = nchoosek(n, k)
Syntax: C = nchoosek(v, k)
nchoosek(n, k) is the number of ways to choose k of n items.
For a vector v, each row of C is one combination of k elements of v, in
lexicographic order of position.
Example: nchoosek([1 2 3], 2)   % [1 2; 1 3; 2 3]
Differences from MATLAB: A scalar result above 2^53 is an error instead of an
inexact value with a warning; the matrix form is limited to 16777216 elements;
numeric, logical and character v only.

## primes
Prime numbers up to n.
Syntax: p = primes(n)
Row vector of the primes not larger than the real scalar n. n below 2 gives a
1-by-0 result.
Example: primes(30)   % 2 3 5 7 11 13 17 19 23 29
Differences from MATLAB: n is limited to 300000000.

## isprime
True for prime elements.
Syntax: tf = isprime(x)
Elementwise test of nonnegative integers; 0 and 1 are not prime.
Example: isprime([2 4 7])   % 1 0 1
Differences from MATLAB: Real double and logical input only.

## factor
Prime factorization.
Syntax: f = factor(n)
f is the row of prime factors of the scalar n, smallest first, repeated by
multiplicity.
factor(1) is 1 and factor(0) is 0. Only one output is supported.
Example: factor(60)   % 2 2 3 5
Differences from MATLAB: n is limited to 2^53; scalar double input only.

## gamma
Gamma function.
Syntax: y = gamma(x)
Elementwise Gamma(x) for real x. Poles at 0 and the negative integers give
Inf; gamma(-Inf) also gives Inf.
Example: gamma(5)   % 24
Differences from MATLAB: Real double and logical input only.

## gammaln
Logarithm of the gamma function.
Syntax: y = gammaln(x)
Elementwise log(Gamma(x)) for real x >= 0 without overflow for large x.
Example: gammaln(10)   % 12.8018
Differences from MATLAB: Real double and logical input only.

## beta
Beta function.
Syntax: B = beta(z, w)
Elementwise B(z, w) = Gamma(z)*Gamma(w)/Gamma(z+w) for z, w >= 0 (one may be a
scalar).
Example: beta(2, 3)   % 0.0833
Differences from MATLAB: Real double and logical input only.

## betaln
Logarithm of the beta function.
Syntax: L = betaln(z, w)
Elementwise log(B(z, w)) for z, w >= 0 (one may be a scalar).
Example: betaln(2, 3)   % -2.4849
Differences from MATLAB: Real double and logical input only.

## erf
Error function.
Syntax: y = erf(x)
Elementwise erf(x) = 2/sqrt(pi) * integral from 0 to x of exp(-t^2).
Example: erf(0.5)   % 0.5205
Differences from MATLAB: Real double and logical input only.

## erfc
Complementary error function.
Syntax: y = erfc(x)
Elementwise 1 - erf(x), computed without cancellation for large x.
Example: erfc(0.5)   % 0.4795
Differences from MATLAB: Real double and logical input only.

## erfinv
Inverse error function.
Syntax: x = erfinv(y)
Elementwise inverse of erf on [-1, 1]. erfinv(1) is Inf, erfinv(-1) is -Inf;
|y| > 1 gives NaN.
Example: erfinv(0.5)   % 0.4769
Differences from MATLAB: Real double and logical input only.

## erfcinv
Inverse complementary error function.
Syntax: x = erfcinv(y)
Elementwise inverse of erfc on [0, 2]. erfcinv(0) is Inf, erfcinv(2) is -Inf;
y outside [0, 2] gives NaN.
Example: erfcinv(0.5)   % 0.4769
Differences from MATLAB: Real double and logical input only. At the smallest
positive subnormal input (5e-324), MatRust returns 27.21329321081295, closer
to the high-precision reference than MATLAB's 27.2164828342302.

## bitand
Bitwise AND.
Syntax: c = bitand(a, b)
Elementwise AND of nonnegative integers up to flintmax (2^53); one input may
be a scalar.
Example: bitand(12, 10)   % 8
Integer arrays use their stored class and retain exact 64-bit values; scalar expansion is supported.
Differences from MATLAB: Typed integer inputs use the two-input form; assumed-type options apply to double inputs.

## bitor
Bitwise OR.
Syntax: c = bitor(a, b)
Elementwise OR of nonnegative integers up to flintmax (2^53); one input may be
a scalar.
Example: bitor(12, 10)   % 14
Integer arrays use their stored class and retain exact 64-bit values; scalar expansion is supported.
Differences from MATLAB: Typed integer inputs use the two-input form; assumed-type options apply to double inputs.

## bitxor
Bitwise exclusive OR.
Syntax: c = bitxor(a, b)
Elementwise XOR of nonnegative integers up to flintmax (2^53); one input may
be a scalar.
Example: bitxor(12, 10)   % 6
Integer arrays use their stored class and retain exact 64-bit values; scalar expansion is supported.
Differences from MATLAB: Typed integer inputs use the two-input form; assumed-type options apply to double inputs.

## bitshift
Shift bits.
Syntax: c = bitshift(a, k)
Syntax: c = bitshift(a, k, assumedtype)
Shifts integer a left by k bits, right if k is negative. assumedtype defaults
to 'uint64'; accepted names are 'uint8', 'uint16', 'uint32', 'uint64', 'int8',
'int16', 'int32', and 'int64'. Bits beyond the type width are dropped; signed
right shifts preserve the sign. Inputs must fit the assumed type and have
absolute value at most 2^53. Results are rounded to double.
Example: bitshift(1, 4)   % 16
Integer arrays use their stored class and retain exact 64-bit values; scalar expansion is supported.
Differences from MATLAB: Typed integer inputs use the two-input form; assumed-type options apply to double inputs.

## bitcmp
Bitwise complement.
Syntax: c = bitcmp(a)
Syntax: c = bitcmp(a, assumedtype)
Flips the bits of integer a in assumedtype (default 'uint64'). Accepted names
are 'uint8', 'uint16', 'uint32', 'uint64', 'int8', 'int16', 'int32', and 'int64'.
Inputs must fit the type and have absolute value at most 2^53. Results are
rounded to double. For the default type this is 2^64 - 1 - a.
Example: bitcmp(0)   % 18446744073709551616 (2^64 after rounding)
Integer input uses its own storage width and returns the same class without rounding.
Example: bitcmp(uint8(0))   % uint8(255)
Differences from MATLAB: Typed integer input takes one argument; no bit-count argument.

## dec2bin
Decimal to binary text.
Syntax: s = dec2bin(d)
Syntax: s = dec2bin(d, len)
Character matrix with one row per element of d (taken column by column),
padded with leading zeros to a common width of at least len digits.
Fractional nonnegative inputs are truncated, as in MATLAB.
Example: dec2bin(5)   % '101'
Inputs range from 0 through 2^53, inclusive.
Differences from MATLAB: Output is limited to 65536 UTF-16 units.

## bin2dec
Binary text to decimal.
Syntax: d = bin2dec(s)
Converts each row of a character array to a number. Spaces are ignored; other
characters that are not 0 or 1 cause an error. Empty text returns a 0-by-0
array; a row containing only spaces returns 0.
Example: bin2dec('101')   % 5
Differences from MATLAB: Character array input only; cell arrays are not
supported yet.

## dec2hex
Decimal to hexadecimal text.
Syntax: s = dec2hex(d)
Syntax: s = dec2hex(d, len)
Uses digits 0-9 and A-F, with one padded row per input element.
Example: dec2hex(255)   % 'FF'
Inputs are nonnegative integers up to and including 2^53.
Differences from MATLAB: Output is limited to 65536 UTF-16 units.

## hex2dec
Hexadecimal text to decimal.
Syntax: d = hex2dec(s)
Converts each row of a character array (upper or lower case) to a number.
Invalid characters, including embedded spaces, cause an error.
Example: hex2dec('FF')   % 255
Differences from MATLAB: Character array input only; cell arrays are not
supported yet.

## dec2base
Decimal to text in any base.
Syntax: s = dec2base(d, base)
Syntax: s = dec2base(d, base, len)
base must be an integer from 2 to 36; a symbol string is not accepted.
Example: dec2base(10, 3)   % '101'
Inputs are nonnegative integers up to and including 2^53.
Differences from MATLAB: Output is limited to 65536 UTF-16 units.

## base2dec
Text in any base to decimal.
Syntax: d = base2dec(s, base)
Converts each row of a character array. base is an integer from 2 to 36 or a
character vector of symbols; invalid characters cause an error.
Example: base2dec('101', 3)   % 10
Differences from MATLAB: Character array input only; cell arrays are not
supported yet.

## nthroot
Real n-th root.
Syntax: y = nthroot(x, n)
Real root of x for real n, with a final correction step for accuracy.
For finite n, negative x needs an odd integer n. For nonnegative x, n = 0
evaluates x.^Inf. Infinite n gives 1 for nonnegative x and -1 for negative x;
NaN in either input gives NaN.
Example: nthroot(-27, 3)   % -3
Differences from MATLAB: Real double and logical input only.

## regexp
Match a regular expression.
Syntax: [startIndex, endIndex] = regexp(str, expression)
Syntax: out = regexp(str, expression, 'match')
Syntax: [tok, mat] = regexp(str, expression, 'tokens', 'match')
Syntax: out = regexp(str, expression, ..., 'once')
Outputs without option strings come in the order start, end, tokenExtents,
match, tokens, names, split. Option strings 'match', 'tokens', 'names',
'start', 'end', 'split' and 'tokenExtents' pick the outputs and their order.
'names' is a 1-by-N struct array, one element per match, with a field per
named token (?<name>...). 'once' returns the first match only, without the
surrounding cell. No match gives 0x0 results. In an alternation, tokens of
the branches the match did not take are left out.
\b is the backspace character; \< and \> match where a word starts and ends.
'ignorecase', 'lineanchors', 'dotexceptnewline', 'freespacing' and
'emptymatch' change how the expression matches.
A cell array str or expression gives a cell array of results of its shape.
\w, \d and \s match ASCII characters only.
Example: regexp('ab12cd345', '\d+', 'match')   % {'12', '345'}
Differences from MATLAB: lookahead, lookbehind, backreferences such as \1,
atomic groups, possessive quantifiers, conditionals and dynamic expressions
are not supported yet and give an error. A lazy quantifier that can match
empty text finds no match: regexp('aaa', 'a*?', 'match') gives {} where
MATLAB gives {'a', 'a', 'a'}.

## regexpi
Match a regular expression, ignoring case.
Syntax: out = regexpi(str, expression, ...)
Same as regexp with the 'ignorecase' option; 'matchcase' turns it off.
Example: regexpi('ABC', 'b', 'match')   % {'B'}
Differences from MATLAB: as for regexp.

## regexprep
Replace text using a regular expression.
Syntax: newStr = regexprep(str, expression, replace)
Syntax: newStr = regexprep(str, expression, replace, 'once')
Syntax: newStr = regexprep(str, expression, replace, 'ignorecase')
Syntax: newStr = regexprep(str, expression, replace, N)
In replace, $0 is the whole match, $1 to $9 are the tokens and $<name> is a
named token; $N past the last token stays as typed. \n, \t, \\ and \$
insert a newline, tab, backslash and dollar.
N replaces only the Nth match. A cell array of expressions applies each in
turn; replace may then be a cell array with one text per expression.
A cell array str gives a cell array of the same shape.
Example: regexprep('hello world', '(\w+) (\w+)', '$2 $1')   % 'world hello'
Differences from MATLAB: 'preservecase' and dynamic replacements ${...} are
not supported yet; the expression limits of regexp apply.

## str2double
Convert text to a double.
Syntax: X = str2double(str)
str is a character row or a cell array. Each text holding one real or
complex number, such as '3.14', '-2.5e3', '1,000', 'Inf' or '1+2i', converts
to that number; anything else gives NaN. A cell array gives an array of its
shape, with NaN for elements that are not text.
Example: str2double({'3.14', 'abc'})   % [3.14 NaN]

## str2num
Convert text holding numbers to a numeric array.
Syntax: X = str2num(str)
Syntax: [X, ok] = str2num(str)
str holds numbers in matrix syntax, such as '1 2 3' or '[1 2; 3 4]'. The
rows of a character matrix are the rows of X. true/false give a logical
result. ok is false and X is [] when the text cannot be converted.
Example: str2num('[1 2; 3 4]')   % [1 2; 3 4]
Differences from MATLAB: the text is not evaluated as code. Only numbers,
true, false, Inf, NaN, pi, eps, i and j, the operators + - * / ^ .* ./ .^,
transposes and colon ranges are accepted; anything else, such as a function
call or a variable name, gives [] and ok = false.

## strjust
Justify a character array.
Syntax: T = strjust(S)
Syntax: T = strjust(S, 'right')
Syntax: T = strjust(S, 'left')
Syntax: T = strjust(S, 'center')
Moves the text of each row to the right (default), left or center by
moving its leading and trailing blanks. S is a character array or a cell
array of them.
Example: strjust('a  ')   % '  a'

## isspace
True for whitespace characters.
Syntax: tf = isspace(str)
A logical array of the size of str: true for space, tab, newline, vertical
tab, form feed and carriage return. Input that is not text gives all false.
Example: isspace('a b')   % [false true false]

## isletter
True for letters.
Syntax: tf = isletter(str)
A logical array of the size of str: true for alphabetic characters.
Input that is not text gives all false.
Example: isletter('a1B')   % [true false true]

## isstrprop
Test characters for a category.
Syntax: tf = isstrprop(str, category)
category is 'alpha', 'alphanum', 'cntrl', 'digit', 'graphic', 'lower',
'print', 'punct', 'wspace', 'upper' or 'xdigit'. A cell array gives a cell
array of logical arrays. Numbers count as character codes. 'digit' and
'xdigit' accept ASCII digits only. 'punct' is Unicode punctuation, so ASCII
symbols such as $ + < = > ^ | ~ are not punctuation.
Example: isstrprop('a1 B', 'digit')   % [false true false false]

## validatestring
Check text against a list of valid texts.
Syntax: validStr = validatestring(str, validStrings)
Syntax: validStr = validatestring(str, validStrings, funcName, varName, argIndex)
Returns the element of validStrings that str matches, ignoring case. str
may be the start of the valid text. When it starts several of them, the
shortest is returned if it starts all the others; otherwise it is an error.
Example: validatestring('app', {'apple', 'banana'})   % 'apple'

## isvarname
True for a valid variable name.
Syntax: tf = isvarname(str)
A valid name starts with a letter, holds only letters, digits and
underscores, has at most 63 characters and is not a keyword.
Example: isvarname('a_1')   % true

## cat
Join arrays along a dimension.
Syntax: C = cat(dim, A, B, ...)
cat(1, ...) stacks arrays vertically, cat(2, ...) joins them side by side.
Numeric, logical and character inputs follow the same rules as [A, B] and [A; B].
Text joined with numbers gives text (cat(2, 'a', 66) is 'aB'); cat(1, 'ab', 'cd') is a
2-by-2 character array. Cell arrays join cell by cell; a non-cell input joined with
cells becomes one cell. cat(dim, A) returns A. Empty inputs are skipped when another
input is nonempty; when every input is empty their sizes must still agree.
Mismatched sizes give "Dimensions of matrices being concatenated are not consistent."
Example: cat(1, [1 2], [3 4])   % [1 2; 3 4]
Differences from MATLAB: dim 1 and 2 only (cat(3, ...) with one nonempty array returns it); N-dimensional
results are not supported yet. The leftmost integer class wins; single wins over double.

## horzcat
Join arrays side by side.
Syntax: C = horzcat(A, B, ...)
Same as [A, B, ...]; cell inputs join as in cat. horzcat() returns [].
Example: horzcat([1 2], [3 4])   % [1 2 3 4]
Differences from MATLAB: 2-D inputs only. Integer and single classes follow the same concatenation rules as bracket literals.

## vertcat
Stack arrays on top of each other.
Syntax: C = vertcat(A, B, ...)
Same as [A; B; ...]; cell inputs join as in cat. vertcat() returns [].
Example: vertcat([1 2], [3 4])   % [1 2; 3 4]
Differences from MATLAB: Two-dimensional inputs only. Integer and single classes follow the same concatenation rules as bracket literals. Cell and struct payloads remain shared.

## rot90
Rotate an array by multiples of 90 degrees.
Syntax: B = rot90(A)
Syntax: B = rot90(A, k)
Rotates counterclockwise by k*90 degrees; k is any integer, negative k turns clockwise.
Numeric, logical, complex, character and cell input. k must be a scalar integer.
Example: rot90([1 2; 3 4])   % [2 4; 1 3]
Differences from MATLAB: N-dimensional input is not supported. Integer and single values retain their storage class.

## squeeze
Remove singleton dimensions.
Syntax: B = squeeze(A)
All arrays here are two-dimensional, so the result is A.
Example: squeeze([1 2; 3 4])   % [1 2; 3 4]
Differences from MATLAB: N-dimensional arrays do not exist yet.

## sub2ind
Convert row/column subscripts to linear indices.
Syntax: ind = sub2ind(siz, I, J)
Syntax: ind = sub2ind(siz, I1, I2, ..., In)
siz is a vector of at least two positive integers. All subscript arrays need the same size.
With fewer subscripts than dimensions the last one spans the remaining dimensions.
As in MATLAB only subscripts below 1 or above the dimension are rejected: 1.5 and NaN
pass through the arithmetic (sub2ind([3 4], 1.5, 1) is 1.5).
Example: sub2ind([3 4], 2, 3)   % 8
Differences from MATLAB: subscripts are numeric or logical double values only
(no integer classes).

## ind2sub
Convert linear indices to subscripts.
Syntax: ind = ind2sub(siz, ind)
Syntax: [I, J] = ind2sub(siz, ind)
Syntax: [I1, I2, ..., In] = ind2sub(siz, ind)
With fewer outputs than dimensions the last output spans the remaining dimensions;
extra outputs are 1. One output returns the linear index itself.
As in MATLAB there is no range check: the last output absorbs any overflow
([r, c] = ind2sub([3 4], 13) gives r = 1, c = 5). siz may be a single number.
Example: [r, c] = ind2sub([3 4], [5 7 12])   % r = [2 1 3], c = [2 3 4]
Differences from MATLAB: none known for double indices.

## magic
Magic square.
Syntax: M = magic(n)
Rows, columns and both diagonals of M sum to n*(n^2+1)/2 for n = 1 and n >= 3.
Odd n uses the de la Loubere (Siamese) method, n divisible by 4 the diagonal-complement
method, and the other even n Strachey's quadrant-swap method (Andrews 1917; Rouse Ball 1939).
magic(2) is [1 3; 4 2] as in MATLAB; n below 1 gives []. Non-integer n is floored.
Example: magic(4)   % [16 2 3 13; 5 11 10 8; 9 7 6 12; 4 14 15 1]
Differences from MATLAB: n is limited by the 16-million-element matrix limit.

## hilb
Hilbert matrix.
Syntax: H = hilb(n)
Syntax: H = hilb(n, 'double')
H(i,j) = 1/(i+j-1).
Example: hilb(3)   % [1 1/2 1/3; 1/2 1/3 1/4; 1/3 1/4 1/5]
Differences from MATLAB: the class 'single' is not supported (no single arrays).

## invhilb
Exact inverse of the Hilbert matrix.
Syntax: H = invhilb(n)
Integer matrix from the closed-form binomial formula (Choi 1983), not by inverting hilb(n).
Entries are exact while they fit integer arithmetic and are otherwise rounded to double;
large n loses accuracy and entries become Inf only where the exact value exceeds double range.
A non-integer n gives "Size inputs must be integers." as in MATLAB.
Example: invhilb(3)   % [9 -36 30; -36 192 -180; 30 -180 180]
Differences from MATLAB: not compared for very large n, where results are rounded approximations.

## pascal
Pascal matrix.
Syntax: P = pascal(n)
Syntax: P = pascal(n, 1)
Syntax: P = pascal(n, 2)
Syntax: P = pascal(n, k, 'double')
pascal(n) is symmetric positive definite with binomial entries. pascal(n,1) is the
lower triangular Cholesky factor (signs alternate). pascal(n,2) is a transposed and
permuted cube root of the identity.
Example: pascal(4)   % [1 1 1 1; 1 2 3 4; 1 3 6 10; 1 4 10 20]
Differences from MATLAB: the class 'single' is not supported (no single arrays).
For n > 28 some entries exceed 2^53 and are rounded; the additive recurrence used
here can then differ in the last digits from the factor product P*P'.

## toeplitz
Toeplitz matrix.
Syntax: T = toeplitz(c, r)
Syntax: T = toeplitz(r)
First column c, first row r. If c(1) and r(1) differ, c(1) wins (no warning).
toeplitz(r) has first row r and, below the diagonal, first column conj(r); it is
symmetric for real r.
Matrix arguments are read column by column; empty arguments, character and logical
arguments are errors, as in MATLAB.
Example: toeplitz([1 2 3], [1 5 6 7])   % [1 5 6 7; 2 1 5 6; 3 2 1 5]
Differences from MATLAB: the class single is not supported.

## vander
Vandermonde matrix.
Syntax: A = vander(c)
Columns are powers of the vector c, highest power first: A(i,j) = c(i)^(m-j) for m = numel(c).
A matrix argument is read column by column. MATLAB's vander takes no second argument.
Example: vander([1 2 3])   % [1 1 1; 4 2 1; 9 3 1]
Differences from MATLAB: real input only.

## hadamard
Hadamard matrix.
Syntax: H = hadamard(n)
n, n/12 or n/20 must be a power of 2 (as in MATLAB; 28 is not accepted).
H has +1/-1 entries and H*H' = n*eye(n).
Example: hadamard(4)   % [1 1 1 1; 1 -1 1 -1; 1 1 -1 -1; 1 -1 -1 1]
Differences from MATLAB: the order is limited to 4096. The 20 block follows the MATLAB
output; the 12 block is the Paley matrix, which equals the MATLAB output for hadamard(12).

## compan
Companion matrix.
Syntax: A = compan(u)
u holds polynomial coefficients, highest power first. The first row of A is
-u(2:end)/u(1) and the subdiagonal is ones, so eig(A) are the roots. u may be complex.
Example: compan([1 8 15])   % [-8 -15; 1 0]
Differences from MATLAB: the class single is not supported.

## rosser
Rosser test matrix.
Syntax: R = rosser
Syntax: R = rosser()
Syntax: R = rosser('double')
8-by-8 symmetric matrix with close eigenvalues; a classic eigenvalue test.
Example: rosser   % first row [611 196 -192 407 -8 -52 -49 29]
Differences from MATLAB: the class 'single' is not supported.

## wilkinson
Wilkinson's eigenvalue test matrix.
Syntax: W = wilkinson(n)
Symmetric tridiagonal: ones beside the diagonal, abs(-(n-1)/2 : (n-1)/2) on it.
Example: wilkinson(5)   % [2 1 0 0 0; 1 1 1 0 0; 0 1 0 1 0; 0 0 1 1 1; 0 0 0 1 2]
Differences from MATLAB: none known.

## randperm
Random permutation.
Syntax: p = randperm(n)
Syntax: p = randperm(n, k)
A row of k distinct integers from 1 to n in random order (k = n by default).
It uses the same stream as rand, so rng(s) makes it repeatable. randperm(n) sorts
n uniform draws. randperm(n,k) samples without replacement; it consumes k draws
when k < n/4, 2*k draws when n/4 <= k < n, and n-1 draws when k = n > 0.
Example: sort(randperm(5))   % [1 2 3 4 5]
Differences from MATLAB: n is capped by MatRust's matrix element limit. Seeded
sequences are not guaranteed to equal MATLAB's for every seed and size.

## accumarray
Group values by subscripts and combine each group.
Syntax: A = accumarray(subs, val)
Syntax: A = accumarray(subs, val, sz)
Syntax: A = accumarray(subs, val, sz, fun)
Syntax: A = accumarray(subs, val, sz, fun, fillval)
subs is a column of positive integers (or an N-by-2 array of row/column subscripts);
val is one value per row of subs, or a scalar. fun combines each group: 'sum' (default),
'max', 'min' or 'prod'. Positions with no value get fillval (0 by default).
max and min ignore NaN. sz may be [] to use max(subs); for a column subs it must be [n 1].
Example: accumarray([1;2;4;2;4], [101;102;103;104;105])   % [101; 206; 0; 208]
Differences from MATLAB: fun is given as text ('max'), since function handles do not exist
yet; no other functions, no cell subs, no sparse output, at most two subscript columns.

## isequal
Test arrays for equality.
Syntax: tf = isequal(A, B)
Syntax: tf = isequal(A, B, C, ...)
True when all inputs have the same size and equal contents. Numeric, logical and
character data compare by value ('a' equals 97); NaN is not equal to NaN; cells compare
element by element; structs compare field by field and the field order does not matter.
Example: isequal({1, 'a'}, {1, 'a'})   % true
Integer and single values are compared inside cells and structs too; 64-bit integers retain exact comparison.
Differences from MATLAB: Function handle comparison is not supported yet.

## cummax
Cumulative maximum.
Syntax: M = cummax(A)
Syntax: M = cummax(A, dim)
Syntax: M = cummax(A, 'reverse')
Syntax: [M, I] = cummax(A)
Running maximum along the first non-singleton dimension, or along dim. Leading NaN
values are kept as they are (complex(1, NaN) stays 1+NaNi); later NaN values are
skipped. I holds the index of each running maximum.
A logical input gives a logical result; character input is an error; complex values
compare by magnitude.
Example: cummax([1 3 2 5 4])   % [1 3 3 5 5]
Differences from MATLAB: no integer or single classes. The second output I is a
MatRust extension; MATLAB accepts only one output. Ties in I take the first index.

## cummin
Cumulative minimum.
Syntax: M = cummin(A)
Syntax: M = cummin(A, dim)
Syntax: M = cummin(A, 'reverse')
Syntax: [M, I] = cummin(A)
Same as cummax with the running minimum.
Example: cummin([5 3 4; 1 6 2], 2)   % [5 3 3; 1 1 1]
Differences from MATLAB: no integer or single classes. The second output I is a
MatRust extension; MATLAB accepts only one output. Ties in I take the first index.

## colon
Colon operator as a function.
Syntax: r = colon(a, b)
Syntax: r = colon(a, s, b)
colon(a,b) is a:b and colon(a,s,b) is a:s:b, a 1-by-n row. An empty range is 1-by-0.
colon('a','e') is the text 'abcde'; a complex operand contributes its real part.
Example: colon(1, 2, 7)   % [1 3 5 7]
Differences from MATLAB: no integer or single classes; non-scalar operands use their
first element.

## flintmax
Largest consecutive integer in double precision.
Syntax: flintmax
Syntax: flintmax('double')
Syntax: flintmax('single')
2^53 = 9007199254740992: every integer up to it is exactly representable.
For 'single' the value is 2^24 = 16777216. Other class names are rejected.
Example: flintmax   % 9.0072e+15
Example: flintmax('single')   % a single precision result
Differences from MATLAB: Only 'double' and 'single' class names are supported.

## realmax
Largest finite double.
Syntax: realmax
Syntax: realmax('double')
Syntax: realmax('single')
About 1.7977e+308. Only a class name is accepted, as in MATLAB (realmax(2) is an error).
Example: realmax   % 1.7977e+308
Example: realmax('single')   % a single precision result
Differences from MATLAB: Only 'double' and 'single' class names are supported.

## realmin
Smallest normalized positive double.
Syntax: realmin
Syntax: realmin('double')
Syntax: realmin('single')
2^-1022, about 2.2251e-308. Only a class name is accepted, as in MATLAB.
Example: realmin   % 2.2251e-308
Example: realmin('single')   % a single precision result
Differences from MATLAB: Only 'double' and 'single' class names are supported.

## xcorr
Cross-correlation and autocorrelation of real vectors.
Syntax: r = xcorr(x)
Syntax: r = xcorr(x, y)
Syntax: r = xcorr(x, maxlag)
Syntax: r = xcorr(x, y, maxlag)
Syntax: r = xcorr(x, y, maxlag, scale)
Syntax: [r, lags] = xcorr(x, maxlag, scale)
The scale option can follow any form. It is 'none' (default), 'biased',
'unbiased' or 'coeff'. Let N be the larger input length. The default maxlag
is N-1; an explicit nonnegative integer gives lags from -maxlag to maxlag.
Positive lag k computes sum(x(j+k)*y(j)); samples outside either input are
zero. Unequal inputs are padded with trailing zeros and require scale 'none'.
'biased' divides by N; 'unbiased' divides by N-abs(lag) within the support
and returns zero outside it. 'coeff' divides by the product of the input
Euclidean lengths. If either input has zero energy, 'coeff' returns NaN at
every lag. With one input, y is x. A scalar second argument means maxlag;
to correlate with a scalar y, supply maxlag as a third argument.
r follows x's row or column orientation; lags is always a row.
Example: [r, lags] = xcorr([1 2 3], 1); r   % 8 14 8
Differences from MATLAB: only nonempty real vectors (logical also accepted);
matrix and complex inputs are not supported yet. Uses direct sums with a
256000000-operation work bound; reduce maxlag for long inputs. Output and
statement memory limits also apply. Large independent lag sums may use the
configured worker pool; small inputs stay serial.

## ode45
Solve nonstiff differential equations, medium order (Dormand-Prince 5(4)).
Syntax: [t, y] = ode45(@(t,y) -y, tspan, y0)
Syntax: [t, y] = ode45(@f, tspan, y0, options)
Syntax: sol = ode45(@f, tspan, y0)
Syntax: [t, y] = ode45('f', tspan, y0)
Syntax: [t, y] = ode45('f', tspan, y0, options)
Syntax: t = ode45('f', tspan, y0)
Integrates dy/dt = f(t, y) from tspan(1) to tspan(end). f can be an anonymous
handle, a named handle, or the text name of a function file (f.m or f.mr in
the Current Folder) or a local function. Anonymous handles capture workspace
variables when created. The function gets t and
the column y and returns a column of the same length.
tspan = [t0 tf] returns the solver steps plus 3 points inside each step
(Refine 4). A longer tspan, increasing or decreasing, returns y at exactly
those times. A single number tf integrates from 0 to tf. Entries of tspan
must be finite.
Each row of y is the solution at the matching time in t.
Defaults: RelTol 1e-3, AbsTol 1e-6, MaxStep (tf - t0)/10. Change them with odeset.
If the tolerances cannot be met, a warning names the time and the result
so far is returned.
With a handle, one output is a solution structure for deval and odextend;
x is a row of solver steps and each column of y is a solution at that step.
Example: [t, y] = ode45(@(t,y) -y, [0 1], 1); y(end)   % 0.3679
Differences from MATLAB: events, extra parameters and the options
Events, Mass, NonNegative, NormControl, OutputFcn and Stats are not supported
yet; they stop with an error. With a function name, one output is t, as in
MATLAB. With no output argument, the single-output result becomes ans instead
of plotting. The solution structure has solver, extdata, x, y, stats and
idata fields like MATLAB's; idata.f3d stores each step's stages as a 2-D
neq-by-(stages*N) matrix instead of a 3-D array. deval and odextend accept it.
NaN or Inf in tspan stops with an error; MATLAB does not reject NaN.

## ode23
Solve nonstiff differential equations, low order (Bogacki-Shampine 3(2)).
Syntax: [t, y] = ode23(@(t,y) -y, tspan, y0)
Syntax: [t, y] = ode23(@f, tspan, y0, options)
Syntax: sol = ode23(@f, tspan, y0)
Syntax: [t, y] = ode23('f', tspan, y0)
Syntax: [t, y] = ode23('f', tspan, y0, options)
Syntax: t = ode23('f', tspan, y0)
Same inputs and outputs as ode45. Each step is cheaper but less accurate,
so ode23 suits loose tolerances. tspan = [t0 tf] returns the solver steps
only (Refine 1).
Example: [t, y] = ode23(@(t,y) -y, [0 1], 1); y(end)   % 0.3679
Differences from MATLAB: events, extra parameters and the options
Events, Mass, NonNegative, NormControl, OutputFcn and Stats are not supported
yet; they stop with an error. With a function name, one output is t, as in
MATLAB. With a handle, one output is the solution structure described under
ode45. With no output argument, the single-output result becomes ans instead
of plotting. deval and odextend accept the structure.
NaN or Inf in tspan stops with an error; MATLAB does not reject NaN.

## odeset
Create or change options for ode45 and ode23.
Syntax: options = odeset('Name', value, ...)
Syntax: options = odeset(oldopts, 'Name', value, ...)
Syntax: options = odeset(oldopts, newopts)
Returns a structure with one field per ODE option, 22 in MATLAB's order, and
[] in every option that is not set. Pass it as the last input of ode45 or
ode23, read a field with options.RelTol or odeget.
ode45 and ode23 use RelTol, AbsTol, MaxStep, InitialStep and Refine. Case
does not matter and a name may be shortened while it stays unique ('reltol',
'Abs'). A value of [] clears that option. In odeset(oldopts, newopts) the
non-empty fields of newopts win.
Function handles are preserved as option values, including Events. Setting
Events stores the handle, but passing it to ode45 or ode23 still gives the
explicit unsupported-option error; event detection is not implemented.
Example: opts = odeset('RelTol', 1e-6, 'Refine', 2); opts.Refine   % 2
Differences from MATLAB: ode45 and ode23 stop with an error when an option
they do not implement is set (Events, Mass, NonNegative, NormControl,
OutputFcn, Stats and the stiff-solver options).

## odeget
Read one option from an odeset option set.
Syntax: v = odeget(options, 'Name')
Syntax: v = odeget(options, 'Name', default)
Returns the value set with odeset, or default ([] when omitted) if that
option is empty. Case does not matter and names may be shortened while
unique. options may be [].
Example: odeget(odeset('RelTol', 1e-6, 'Refine', 2), 'Refine')   % 2
Differences from MATLAB: none known.

## sinh
Hyperbolic sine.
Syntax: y = sinh(X)
Applies elementwise to a real or complex double array; the result has the size of X.
Example: sinh(1)   % 1.1752
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## cosh
Hyperbolic cosine.
Syntax: y = cosh(X)
Applies elementwise to a real or complex double array; the result has the size of X.
Example: cosh([0 1])   % [1 1.5431]
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## tanh
Hyperbolic tangent.
Syntax: y = tanh(X)
Applies elementwise to a real or complex double array; the result has the size of X.
Example: tanh(0.5)   % 0.4621
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## asinh
Inverse hyperbolic sine.
Syntax: y = asinh(X)
Applies elementwise to a real or complex double array; the result has the size of X.
Example: asinh(3)   % 1.8184
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acosh
Inverse hyperbolic cosine.
Syntax: y = acosh(X)
Applies elementwise to a real or complex double array; the result has the size of X. Real input outside the real domain gives the complex principal value, as in MATLAB.
Example: acosh(0.5)   % 0.0000 + 1.0472i
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## atanh
Inverse hyperbolic tangent.
Syntax: y = atanh(X)
Applies elementwise to a real or complex double array; the result has the size of X. Real input outside the real domain gives the complex principal value, as in MATLAB.
Example: atanh(2)   % 0.5493 + 1.5708i
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## sec
Secant of argument in radians.
Syntax: y = sec(X)
Computed as 1./cos(X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: sec(pi/3)   % 2.0000
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## csc
Cosecant of argument in radians.
Syntax: y = csc(X)
Computed as 1./sin(X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: csc(pi/6)   % 2.0000
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## cot
Cotangent of argument in radians.
Syntax: y = cot(X)
Computed as 1./tan(X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: cot(pi/4)   % 1.0000
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## asec
Inverse secant in radians.
Syntax: y = asec(X)
Computed as acos(1./X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: asec(2)   % 1.0472
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acsc
Inverse cosecant in radians.
Syntax: y = acsc(X)
Computed as asin(1./X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: acsc(2)   % 0.5236
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acot
Inverse cotangent in radians.
Syntax: y = acot(X)
Computed as atan(1./X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: acot(1)   % 0.7854
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## sech
Hyperbolic secant.
Syntax: y = sech(X)
Computed as 1./cosh(X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: sech(1)   % 0.6481
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## csch
Hyperbolic cosecant.
Syntax: y = csch(X)
Computed as 1./sinh(X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: csch(1)   % 0.8509
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## coth
Hyperbolic cotangent.
Syntax: y = coth(X)
Computed as 1./tanh(X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: coth(1)   % 1.3130
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## asech
Inverse hyperbolic secant.
Syntax: y = asech(X)
Computed as acosh(1./X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: asech(0.5)   % 1.3170
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acsch
Inverse hyperbolic cosecant.
Syntax: y = acsch(X)
Computed as asinh(1./X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: acsch(1)   % 0.8814
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acoth
Inverse hyperbolic cotangent.
Syntax: y = acoth(X)
Computed as atanh(1./X), as in MATLAB, elementwise on a real or complex double array. Out-of-domain real input gives a complex result.
Example: acoth(2)   % 0.5493
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## sind
Sine of argument in degrees.
Syntax: y = sind(X)
The angle is first reduced in degrees (rem(X,360) is exact), so multiples of 90 give exact 0, 1, -1 or Inf: sind(180) is exactly 0 and tand(90) is Inf. Inf and NaN give NaN.
Example: sind([0 30 90 180])   % [0 0.5000 1.0000 0]
Differences from MATLAB: Real double arrays only; complex, character, logical and cell input stop with an error. Single input is not supported yet.

## cosd
Cosine of argument in degrees.
Syntax: y = cosd(X)
The angle is first reduced in degrees (rem(X,360) is exact), so multiples of 90 give exact 0, 1, -1 or Inf: sind(180) is exactly 0 and tand(90) is Inf. Inf and NaN give NaN.
Example: cosd([0 60 90])   % [1.0000 0.5000 0]
Differences from MATLAB: Real double arrays only; complex, character, logical and cell input stop with an error. Single input is not supported yet.

## tand
Tangent of argument in degrees.
Syntax: y = tand(X)
The angle is first reduced in degrees (rem(X,360) is exact), so multiples of 90 give exact 0, 1, -1 or Inf: sind(180) is exactly 0 and tand(90) is Inf. Inf and NaN give NaN.
Example: tand([0 45 90])   % [0 1 Inf]
Differences from MATLAB: Real double arrays only; complex, character, logical and cell input stop with an error. Single input is not supported yet.

## secd
Secant of argument in degrees.
Syntax: y = secd(X)
The angle is first reduced in degrees (rem(X,360) is exact), so multiples of 90 give exact 0, 1, -1 or Inf: sind(180) is exactly 0 and tand(90) is Inf. Inf and NaN give NaN.
Example: secd(60)   % 2.0000
Differences from MATLAB: Real double arrays only; complex, character, logical and cell input stop with an error. Single input is not supported yet.

## cscd
Cosecant of argument in degrees.
Syntax: y = cscd(X)
The angle is first reduced in degrees (rem(X,360) is exact), so multiples of 90 give exact 0, 1, -1 or Inf: sind(180) is exactly 0 and tand(90) is Inf. Inf and NaN give NaN.
Example: cscd(30)   % 2.0000
Differences from MATLAB: Real double arrays only; complex, character, logical and cell input stop with an error. Single input is not supported yet.

## cotd
Cotangent of argument in degrees.
Syntax: y = cotd(X)
The angle is first reduced in degrees (rem(X,360) is exact), so multiples of 90 give exact 0, 1, -1 or Inf: sind(180) is exactly 0 and tand(90) is Inf. Inf and NaN give NaN.
Example: cotd([45 90])   % [1 0]
Differences from MATLAB: Real double arrays only; complex, character, logical and cell input stop with an error. Single input is not supported yet.

## asind
Inverse sine in degrees.
Syntax: y = asind(X)
Computed as 180/pi*asin(X) on a real or complex double array; out-of-domain real input gives a complex result (asind(2) is 90.0000 - 75.4561i).
Example: asind(0.5)   % 30.0000
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acosd
Inverse cosine in degrees.
Syntax: y = acosd(X)
Computed as 180/pi*acos(X) on a real or complex double array; out-of-domain real input gives a complex result (asind(2) is 90.0000 - 75.4561i).
Example: acosd(0.5)   % 60.0000
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## atand
Inverse tangent in degrees.
Syntax: y = atand(X)
Computed as 180/pi*atan(X) on a real or complex double array; out-of-domain real input gives a complex result (asind(2) is 90.0000 - 75.4561i).
Example: atand([1 Inf])   % [45 90]
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## asecd
Inverse secant in degrees.
Syntax: y = asecd(X)
Computed as 180/pi*asec(X) on a real or complex double array; out-of-domain real input gives a complex result (asind(2) is 90.0000 - 75.4561i).
Example: asecd(2)   % 60.0000
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acscd
Inverse cosecant in degrees.
Syntax: y = acscd(X)
Computed as 180/pi*acsc(X) on a real or complex double array; out-of-domain real input gives a complex result (asind(2) is 90.0000 - 75.4561i).
Example: acscd(2)   % 30.0000
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## acotd
Inverse cotangent in degrees.
Syntax: y = acotd(X)
Computed as 180/pi*acot(X) on a real or complex double array; out-of-domain real input gives a complex result (asind(2) is 90.0000 - 75.4561i).
Example: acotd(1)   % 45
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## deg2rad
Convert angles from degrees to radians.
Syntax: R = deg2rad(D)
Returns (pi/180)*D for a real or complex double array.
Example: deg2rad(180)   % 3.1416
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## rad2deg
Convert angles from radians to degrees.
Syntax: D = rad2deg(R)
Returns (180/pi)*R for a real or complex double array.
Example: rad2deg(pi/2)   % 90
Differences from MATLAB: Character, logical and cell input stop with an error, as MATLAB's sinh does; single input is not supported yet.

## cart2pol
Cartesian to polar or cylindrical coordinates.
Syntax: [theta, rho] = cart2pol(x, y)
Syntax: [theta, rho, z] = cart2pol(x, y, z)
theta is atan2(y,x) and rho is hypot(x,y). Inputs must have the same size, or be scalar. z is passed through unchanged.
Example: [t, r] = cart2pol(1, 1); [t r]   % [0.7854 1.4142]
Differences from MATLAB: Real double inputs only; complex, character, logical and cell input stop with an error.

## pol2cart
Polar or cylindrical to Cartesian coordinates.
Syntax: [x, y] = pol2cart(theta, rho)
Syntax: [x, y, z] = pol2cart(theta, rho, z)
x is rho.*cos(theta) and y is rho.*sin(theta). Inputs must have the same size, or be scalar. z is passed through unchanged.
Example: [x, y] = pol2cart(pi/2, 2); [x y]   % [0.0000 2.0000]
Differences from MATLAB: Real double inputs only; complex, character, logical and cell input stop with an error.

## cart2sph
Cartesian to spherical coordinates.
Syntax: [azimuth, elevation, r] = cart2sph(x, y, z)
azimuth is atan2(y,x), elevation is atan2(z,hypot(x,y)) and r is the distance from the origin. Inputs must have the same size, or be scalar.
Example: [az, el, r] = cart2sph(1, 1, 1); [az el r]   % [0.7854 0.6155 1.7321]
Differences from MATLAB: Real double inputs only; complex, character, logical and cell input stop with an error.

## sph2cart
Spherical to Cartesian coordinates.
Syntax: [x, y, z] = sph2cart(azimuth, elevation, r)
x is r.*cos(elevation).*cos(azimuth), y is r.*cos(elevation).*sin(azimuth) and z is r.*sin(elevation). Inputs must have the same size, or be scalar.
Example: [x, y, z] = sph2cart(0, 0, 1); [x y z]   % [1 0 0]
Differences from MATLAB: Real double inputs only; complex, character, logical and cell input stop with an error.

## datenum
Serial date number.
Syntax: n = datenum(Y, M, D)
Syntax: n = datenum(Y, M, D, H, MI, S)
Syntax: n = datenum(V)
Syntax: n = datenum(text, format)
Syntax: n = datenum(text)
Day 1 is 1-Jan-0000 in the proleptic Gregorian calendar; the time of day is the
fraction. A month or day out of range rolls over: datenum(2016, 13, 1) is
1-Jan-2017 and datenum(2016, 1, 0) is 31-Dec-2015. Scalar arguments combine
with arrays of one common size. V is a date vector or a matrix of them (3 or 6
columns); the result is a column. Text is read with the format (the tokens of
datestr), or without one in the forms 'dd-mmm-yyyy HH:MM:SS', 'dd-mmm-yyyy',
'mm/dd/yy', 'mm/dd/yyyy', 'mmm.dd,yyyy HH:MM:SS', 'mmm.dd,yyyy' and
'yyyy-mm-dd HH:MM:SS'. A numeric second argument after text is the pivot year
for two-digit years (default: the current year less 50).
Example: datenum(2016, 3, 1)   % 736390
Example: datenum('2016-03-01 12:30', 'yyyy-mm-dd HH:MM')   % 7.3639e+05
Differences from MATLAB: Date text without a year (such as
'15:00', which MATLAB places in the current year) is rejected; cell arrays of
text are not supported yet.

## datestr
Date as text.
Syntax: s = datestr(D)
Syntax: s = datestr(D, format)
Syntax: s = datestr(D, code)
D holds serial date numbers, date vectors (6 columns whose rows look like
dates) or date text. Without a format the result is 'dd-mmm-yyyy' when every
date is a whole day and 'dd-mmm-yyyy HH:MM:SS' otherwise. Format tokens: yyyy
yy mmmm mmm mm m dddd ddd dd d HH MM SS FFF AM PM QQ; other characters are
copied. With AM or PM the hour is 12-hour and blank-padded. code is MATLAB's
format number 0 to 31 (1 'dd-mmm-yyyy', 29 'yyyy-mm-dd', 31 'yyyy-mm-dd
HH:MM:SS', ...). The time is rounded to the millisecond when the format shows
FFF and to the second otherwise. Several dates give a character matrix, one
row each.
Example: datestr(736390.75, 'yyyy-mm-dd HH:MM:SS.FFF')   % 2016-03-01 18:00:00.000
Example: datestr([736390 736391])   % 01-Mar-2016 / 02-Mar-2016
Differences from MATLAB: The pivot-year argument and the 'local'
option are not supported yet.

## datevec
Date vector.
Syntax: V = datevec(D)
Syntax: V = datevec(text, format)
Syntax: [Y, M, D, H, MI, S] = datevec(...)
Returns one row [year month day hour minute second] per date: per element of a
serial-number array (column-major order) or per row of date text. Text without
a format is read as by datenum. With several outputs each output is a column of
the matrix. Seconds are rounded to the millisecond.
Example: datevec(736390.5)   % 2016 3 1 12 0 0
Example: [y, m, d] = datevec('2016-03-01', 'yyyy-mm-dd')   % y = 2016, m = 3, d = 1
Differences from MATLAB: The millisecond rounding of the seconds
was not checked against MATLAB.

## weekday
Day of the week.
Syntax: n = weekday(D)
Syntax: [n, s] = weekday(D)
Syntax: [n, s] = weekday(D, 'long')
n is 1 for Sunday to 7 for Saturday, one per serial date number (the shape of
D) or per row of date text. s holds the day names, three letters or in full
with 'long', one row per date.
Example: [n, s] = weekday(736390)   % n = 3, s = 'Tue'
Differences from MATLAB: none known.

## etime
Seconds between two date vectors.
Syntax: e = etime(T1, T0)
T1 and T0 are date vectors with 6 columns and the same number of rows; e holds
T1 - T0 in seconds, one row each.
Example: etime([2016 3 1 0 1 30.5], [2016 3 1 0 0 0])   % 90.5000
Differences from MATLAB: none known.

## addtodate
Add a quantity to serial date numbers.
Syntax: R = addtodate(D, Q, field)
field is 'year', 'month', 'day', 'hour', 'minute', 'second' or 'millisecond';
Q is a scalar or has the size of D. Adding years or months keeps the day of the
month and the time, limited to the last day of the new month, so 31-Jan-2016
plus one month is 29-Feb-2016; Q must then be an integer.
Example: addtodate(736390, 1, 'month')   % 736421
Differences from MATLAB: none known.

## eomday
Last day of the month.
Syntax: E = eomday(Y, M)
Returns the number of days in month M of year Y (Gregorian leap years).
Scalar arguments combine with arrays of one common size.
Example: eomday([2015 2016 2100], 2)   % [28 29 28]
Differences from MATLAB: none known.

## calendar
Calendar matrix of a month.
Syntax: C = calendar(Y, M)
Syntax: C = calendar(D)
Returns a 6-by-7 matrix of the days of the month, weeks in rows starting on
Sunday, zeros elsewhere. D is a serial date number or date text.
Example: C = calendar(2016, 2); C(1, :)   % 0 1 2 3 4 5 6
Differences from MATLAB: Without an output argument the matrix is shown as ans; MATLAB prints a
titled calendar instead. calendar with no argument (the current month) is not

## cov
Covariance.
Syntax: C = cov(x)
Syntax: C = cov(x, y)
Syntax: C = cov(x, flag)
Syntax: C = cov(x, y, flag)
For a vector it returns the variance. For a matrix, each row is an
observation and each column a variable; the result is the covariance matrix
of the columns. cov(x, y) treats x(:) and y(:) as two variables and returns
a 2-by-2 matrix. flag 0 (default) divides by N-1, flag 1 by N; one
observation divides by 1. cov([]) is NaN.
Example: cov([1 2; 3 4; 5 7])   % [4 5; 5 6.3333]
Differences from MATLAB: complex input is not supported yet.

## corrcoef
Correlation coefficients.
Syntax: R = corrcoef(x)
Syntax: R = corrcoef(x, y)
Columns of x are variables and rows observations; corrcoef(x, y) uses x(:)
and y(:). The diagonal is exactly 1, off-diagonal values are limited to
[-1, 1], and a constant variable gives NaN.
Example: corrcoef([1 2 3 4], [2 4 6 9])   % [1 0.9944; 0.9944 1]
Differences from MATLAB: the p-value and bound outputs and the 'alpha' and
'rows' options are not supported yet; complex input is not supported yet.

## movmean
Moving mean.
Syntax: M = movmean(A, k)
Syntax: M = movmean(A, [kb kf])
Syntax: M = movmean(A, k, dim, nanflag, 'Endpoints', e)
Each element is the mean of a window along dim (default: the first
non-singleton dimension). An odd k is centred; an even k takes k/2 elements
before and k/2-1 after. [kb kf] takes kb before and kf after. nanflag is
'includenan' (default, any NaN in the window gives NaN) or 'omitnan'.
'Endpoints' is 'shrink' (default: windows are cut at the ends), 'discard'
(only full windows are returned), 'fill' (pad missing samples with NaN)
or a numeric value to pad missing samples before reduction. 'omitnan'
ignores NaN padding as well as NaN samples.
Example: movmean([1 2 3 4 5], 3)   % [1.5 2 3 4 4.5]
Differences from MATLAB: k must be an integer; 'SamplePoints', dim 3 or
higher, and complex input are not supported yet and stop with an error.

## movsum
Moving sum.
Syntax: S = movsum(A, k)
Syntax: S = movsum(A, [kb kf])
Syntax: S = movsum(A, k, dim, nanflag, 'Endpoints', e)
Same windows and options as movmean. With 'omitnan' a window of only NaN
sums to 0.
Example: movsum([1 2 3 4 5], [2 0])   % [1 3 6 9 12]
Differences from MATLAB: k must be an integer; 'SamplePoints', dim 3 or
higher, and complex input are not supported yet and stop with an error.

## detrend
Remove a linear trend or the mean.
Syntax: y = detrend(x)
Syntax: y = detrend(x, 'linear')
Syntax: y = detrend(x, 'constant')
Removes the least-squares line ('linear', the default, or 1) or the mean
('constant', or 0) from a vector, or from each column of a matrix.
Example: detrend([1 2 4 7])   % [0.5 -0.5 -0.5 0.5]
Differences from MATLAB: breakpoints (a third argument) are not supported
yet; complex input is not supported yet.

## discretize
Bin indices of data.
Syntax: Y = discretize(X, edges)
Syntax: Y = discretize(X, edges, values)
Syntax: Y = discretize(X, edges, 'IncludedEdge', side)
Syntax: [Y, E] = discretize(...)
Y has the size of X and holds the bin of each element: bin i is
[edges(i), edges(i+1)), and the last bin also holds edges(end). With
'IncludedEdge', 'right' bin i is (edges(i), edges(i+1)] and the first bin
also holds edges(1). Values outside the edges, and NaN, give NaN. values
replaces bin i by values(i). E returns the edges.
Example: discretize([1 2.5 3 4 7], [1 3 5])   % [1 1 2 2 NaN]
Differences from MATLAB: a number of bins instead of edges, and
'categorical', are not supported yet.

## hist
Histogram bin counts.
Syntax: n = hist(y)
Syntax: n = hist(y, nbins)
Syntax: n = hist(y, centers)
Syntax: [n, c] = hist(...)
Counts y in nbins (default 10) equally spaced bins between min(y) and
max(y), or in bins around the given centers. A value on the boundary
between two bins goes to the lower bin. For a matrix each column is binned
separately and n has one column per column of y. c returns the bin centers.
Example: n = hist([1 2 2 3 3 3], 3)   % [1 2 3]
Differences from MATLAB: hist with no output argument draws a bar chart in
MATLAB; here it stops with an error ("plotting is not supported yet").

## issorted
True if the data are in ascending order.
Syntax: tf = issorted(A)
Syntax: tf = issorted(A, 'rows')
A may be a numeric, logical or char vector, or a cell array of character
vectors. NaN sorts after every number. With 'rows' the rows of a matrix are
compared as in sortrows.
Example: issorted([1 2 2 3])   % true
Differences from MATLAB: complex input is not supported yet.

## range
Range of values (maximum minus minimum).
Syntax: r = range(X)
Syntax: r = range(X, dim)
Returns max(X) - min(X) along the first non-singleton dimension, or along
dim. NaN values are ignored, as in max and min.
Example: range([4 1 9])   % 8
Differences from MATLAB: dim 3 or higher and complex input are not
supported yet.

## cell2mat
Convert a cell array of matrices to one matrix.
Syntax: M = cell2mat(C)
Concatenates the contents of C: each row of cells side by side, then the rows
on top of each other. All contents must have the same class (double, logical,
char or struct), and the pieces must fit together. An empty cell gives [].
Example: cell2mat({[1 2], 3; [4 5], 6})   % [1 2 3; 4 5 6]
Differences from MATLAB: 2-D cell arrays only. Error messages match MATLAB's.

## cellstr
Convert a character array to a cell array of character vectors.
Syntax: C = cellstr(S)
Each row of the char matrix S becomes one cell, with trailing whitespace
removed; the result is a column. A cell array of character vectors is returned
unchanged. cellstr('') is {''}.
Example: cellstr(vertcat('ab ', 'cd '))   % {'ab'; 'cd'}
Differences from MATLAB: string arrays are not supported.

## deal
Copy inputs to outputs.
Syntax: [A, B, ...] = deal(X)
Syntax: [A, B, ...] = deal(X1, X2, ...)
With one input every output gets a copy of it; otherwise the number of inputs
must equal the number of outputs and each output gets its input.
Example: [a, b] = deal(1, 2); [a b]   % [1 2]
Differences from MATLAB: none known.

## structfun
Apply a function to each field of a scalar struct.
Syntax: A = structfun(f, S)
Syntax: A = structfun(f, S, 'UniformOutput', false)
Syntax: A = structfun(f, S, 'ErrorHandler', g)
Calls f on each field value in field order. By default every call must
return a scalar of one class and the results form a column with one row per
field. With 'UniformOutput', false the results are returned as a struct with
the same field names. With 'ErrorHandler', g(err, value) supplies the result
for a field whose call fails; err has the fields message, identifier and
index. Several outputs are supported.
Example: s.a = 1; s.b = 2; structfun(@(v) v * 2, s)   % [2; 4]
Differences from MATLAB: uniform results must be numeric, logical or char
scalars.

## strtok
First token of a character vector.
Syntax: token = strtok(str)
Syntax: [token, remainder] = strtok(str, delimiters)
Skips leading delimiters and returns the text up to the next delimiter; the
remainder starts at that delimiter. The default delimiters are whitespace
(space, tab, newline, vertical tab, form feed, carriage return). For a cell
array of character vectors both outputs are cell arrays of the same size.
Example: [t, r] = strtok('a,b,c', ',')   % t = 'a', r = ',b,c'
Differences from MATLAB: string arrays are not supported.

## assert
Throw an error when a condition is false.
Syntax: assert(cond)
Syntax: assert(cond, msg, A1, ...)
Syntax: assert(cond, id, msg, A1, ...)
cond must be a logical or numeric scalar. When it is false, assert raises
"Assertion failed." or the message, formatted like error() with the extra
arguments; an identifier is kept for catch.
Example: x = 3; assert(x > 0, 'x must be positive')   % no output
Differences from MATLAB: none known.

## warning
Display a warning and continue.
Syntax: warning(msg)
Syntax: warning(msg, A1, ...)
Syntax: warning(id, msg, A1, ...)
Syntax: warning('off', id)
Syntax: warning('on', 'all')
Prints "Warning: " and the message (formatted like error() when there are
extra arguments) and continues. warning('off', id) silences that identifier
for the session, warning('off', 'all') silences all warnings, and 'on' turns
them back on.
Example: warning('Value %d is large', 7)   % Warning: Value 7 is large
Differences from MATLAB: the warning is one line without the "> In" location
lines; querying the state (warning('query', ...), s = warning) and
'backtrace' or 'verbose' are not supported yet.

## eval
Run MATLAB code held in text.
Syntax: eval(expression)
Syntax: eval(expression, catchExpression)
Syntax: x = eval(expression)
Runs the text in the current workspace, so variables it assigns stay. When
an output is requested, a text that is one expression returns its value and
the outer statement controls display. Otherwise, statements inside the text
control display, including their semicolons. With catchExpression, that text
runs instead when the first one fails.
Example: eval('y = 2 + 3;'); y   % 5
Differences from MATLAB: function definitions are rejected.
Syntax errors other than an incomplete statement
keep MatRust's parser wording after "Error: ".

## rat
Rational approximation.
Syntax: S = rat(X)
Syntax: [N, D] = rat(X)
Syntax: [N, D] = rat(X, tol)
With two outputs, N./D approximates X to within tol, by default
1e-6*norm(X(:),1). With one output, the continued fraction as text, one row
per element (for example 3 + 1/(7 + 1/(16)) for pi).
Example: [n, d] = rat(0.75)   % n = 3, d = 4
Differences from MATLAB: real input only.

## rats
Rational approximation as fixed-width text.
Syntax: S = rats(X)
Syntax: S = rats(X, strlen)
Each element becomes N/D (or N when D is 1), centred in a field of strlen+1
characters (strlen is 13 by default); a row of X is one row of text. The
tolerance is 1e-6*norm(X(:),1). An element that does not fit prints as *.
Example: rats(0.75)   % '      3/4     '
Differences from MATLAB: real input only. Widths other than the default and the * rule
are not checked yet.

## integral
Numerical integration of a function of one variable.
Syntax: q = integral(fun, a, b)
Syntax: q = integral(fun, a, b, 'Name', value, ...)
fun is a function handle called with a row of points; it must return a row of the same size. a and b are real scalars and may be -Inf or Inf. Options: 'AbsTol' (default 1e-10), 'RelTol' (default 1e-6), 'Waypoints' (real points where fun has kinks or jumps) and 'ArrayValued' (true calls fun with one point at a time and integrates every element of the array it returns). Complex integrands give a complex result. Adaptive Gauss-Kronrod (7,15) quadrature with the endpoint and infinite-interval transformations of Shampine (2008).
Example: integral(@(x) x.^2, 0, 1)   % 0.3333
Example: integral(@(x) exp(-x), 0, Inf)   % 1
Example: integral(@(x) [x; x.^2], 0, 1, 'ArrayValued', true)   % [0.5000; 0.3333]
Example: integral(@(x) abs(x - 1), 0, 3, 'Waypoints', 1)   % 2.5000
Differences from MATLAB: Complex limits and complex waypoints (contour integration) are not supported yet. Warnings use MATLAB's wording.

## integral2
Numerical integration over a planar region.
Syntax: q = integral2(fun, xmin, xmax, ymin, ymax)
Syntax: q = integral2(fun, xmin, xmax, ymin, ymax, 'AbsTol', tol, 'RelTol', tol)
fun(x, y) is called with arrays of the same size and must return an array of that size. xmin and xmax are real scalars; ymin and ymax are real scalars or function handles of x. Any limit may be infinite. Defaults: 'AbsTol' 1e-10, 'RelTol' 1e-6.
Example: integral2(@(x, y) x .* y, 0, 1, 0, 2)   % 1.0000
Example: integral2(@(x, y) x.^2 + y.^2, 0, 1, 0, @(x) x)   % 0.3333
Differences from MATLAB: Always computes iterated one-dimensional integrals; 'Method' is accepted but has no effect. Uses tighter internal tolerances, scales inner absolute tolerances by the outer integration measure, and propagates inner error estimates. The last digits and warnings can differ from MATLAB's tiled method.

## integral3
Numerical integration over a three-dimensional region.
Syntax: q = integral3(fun, xmin, xmax, ymin, ymax, zmin, zmax)
Syntax: q = integral3(fun, xmin, xmax, ymin, ymax, zmin, zmax, 'AbsTol', tol, 'RelTol', tol)
fun(x, y, z) is called with arrays of the same size and must return an array of that size. ymin and ymax may be function handles of x; zmin and zmax may be function handles of (x, y). Limits may be infinite. Defaults: 'AbsTol' 1e-10, 'RelTol' 1e-6.
Example: integral3(@(x, y, z) x + y + z, 0, 1, 0, 1, 0, 1)   % 1.5000
Differences from MATLAB: Iterated one-dimensional integrals with tighter internal tolerances, scaling of inner absolute tolerances by the outer integration measure, and propagation of inner error estimates; 'Method' is accepted but has no effect. The last digits can differ from MATLAB.

## quad
Numerical integration by adaptive Simpson quadrature.
Syntax: q = quad(fun, a, b)
Syntax: q = quad(fun, a, b, tol)
Syntax: q = quad(fun, a, b, tol, [], p1, p2, ...)
Syntax: [q, fcnt] = quad(...)
fun is a function handle or a function name called with a row of points (and p1, p2, ... after it). The default absolute tolerance is 1e-6. fcnt is the number of integrand values used. Limits must be finite scalars.
Example: quad(@(x) exp(x), 0, 1)   % 1.7183
Differences from MATLAB: A nonzero trace argument is not supported yet.

## quadgk
Numerical integration by adaptive Gauss-Kronrod quadrature.
Syntax: q = quadgk(fun, a, b)
Syntax: q = quadgk(fun, a, b, 'Name', value, ...)
Syntax: [q, errbnd] = quadgk(...)
fun is a function handle called with a row of points; it must return a row of the same size. a and b may be infinite, and fun may have integrable singularities at the ends. Options: 'AbsTol' (default 1e-10), 'RelTol' (default 1e-6), 'Waypoints' and 'MaxIntervalCount' (default 650). errbnd is an approximate bound on the absolute error.
Example: quadgk(@(x) exp(-x.^2), -Inf, Inf)   % 1.7725
Example: [q, err] = quadgk(@(x) 1 ./ sqrt(x), 0, 1)   % q = 2.0000, err = 2.5601e-13
Differences from MATLAB: Complex limits and waypoints (contour integration) are not supported yet.

## dblquad
Double integral over a rectangle by nested quad.
Syntax: q = dblquad(fun, xmin, xmax, ymin, ymax)
Syntax: q = dblquad(fun, xmin, xmax, ymin, ymax, tol)
fun(x, y) is called with a row x and a scalar y and must return a row the size of x. The default tolerance is 1e-6.
Example: dblquad(@(x, y) x .* y, 0, 1, 0, 2)   % 1.0000
Differences from MATLAB: Only the default method @quad is supported.

## triplequad
Triple integral over a box by nested quad.
Syntax: q = triplequad(fun, xmin, xmax, ymin, ymax, zmin, zmax)
Syntax: q = triplequad(fun, xmin, xmax, ymin, ymax, zmin, zmax, tol)
fun(x, y, z) is called with a row x and scalars y and z and must return a row the size of x. The default tolerance is 1e-6.
Example: triplequad(@(x, y, z) x .* y .* z, 0, 1, 0, 1, 0, 1)   % 0.1250
Differences from MATLAB: Only the default method @quad is supported.

## acos
Inverse cosine in radians.
Syntax: Y = acos(X)
Real input in [-1, 1] gives values in [0, pi]. Real input outside [-1, 1]
and complex input give the principal complex value.
Example: acos(0.5)   % 1.0472
Example: acos(2)   % 0 + 1.3170i
Differences from MATLAB: double, logical and char 2-D arrays only.

## asin
Inverse sine in radians.
Syntax: Y = asin(X)
Real input in [-1, 1] gives values in [-pi/2, pi/2]. Real input outside
[-1, 1] and complex input give the principal complex value.
Example: asin(1)   % 1.5708
Example: asin(2)   % 1.5708 - 1.3170i
Differences from MATLAB: double, logical and char 2-D arrays only.

## atan
Inverse tangent in radians.
Syntax: Y = atan(X)
Real input gives values in [-pi/2, pi/2]; complex input gives the principal
value. See atan2 for the four-quadrant form.
Example: atan(1)   % 0.7854
Example: atan(1+1i)   % 1.0172 + 0.4024i
Differences from MATLAB: double, logical and char 2-D arrays only.

## realsqrt
Square root of nonnegative real input.
Syntax: Y = realsqrt(X)
Stops with an error when an element is negative instead of returning a
complex result.
Example: realsqrt(16)   % 4
Differences from MATLAB: complex input gives an error.

## reallog
Natural logarithm of nonnegative real input.
Syntax: Y = reallog(X)
Stops with an error when an element is negative instead of returning a
complex result.
Example: reallog(1)   % 0
Differences from MATLAB: complex input gives an error.

## realpow
Array power with real output.
Syntax: Z = realpow(X, Y)
Same as X.^Y, but stops with an error when an element of the result would be
complex (a negative base with a non-integer exponent).
Example: realpow(2, 3)   % 8
Differences from MATLAB: complex input gives an error.

## plus
Addition, the function form of a + b.
Syntax: C = plus(A, B)
Evaluates a + b with the operator's own rules.
Example: plus(1, [2 3])   % [3 4]
Differences from MATLAB: same limits as the + operator.

## minus
Subtraction, the function form of a - b.
Syntax: C = minus(A, B)
Evaluates a - b with the operator's own rules.
Example: minus([5 6], 1)   % [4 5]
Differences from MATLAB: same limits as the - operator.

## times
Elementwise multiplication, the function form of a .* b.
Syntax: C = times(A, B)
Evaluates a .* b with the operator's own rules.
Example: times([1 2], [3 4])   % [3 8]
Differences from MATLAB: same limits as the .* operator.

## rdivide
Elementwise right division, the function form of a ./ b.
Syntax: C = rdivide(A, B)
Evaluates a ./ b with the operator's own rules.
Example: rdivide(1, [2 4])   % [0.5 0.25]
Differences from MATLAB: same limits as the ./ operator.

## ldivide
Elementwise left division, the function form of a .\ b.
Syntax: C = ldivide(A, B)
Evaluates b ./ a, which is what a .\ b means.
Example: ldivide(2, [4 8])   % [2 4]
Differences from MATLAB: the .\ operator itself is not parsed yet.

## mtimes
Matrix multiplication, the function form of a * b.
Syntax: C = mtimes(A, B)
Evaluates a * b with the operator's own rules.
Example: mtimes([1 2], [3; 4])   % 11
Differences from MATLAB: same limits as the * operator.

## power
Elementwise power, the function form of a .^ b.
Syntax: C = power(A, B)
Evaluates a .^ b; a negative base with a non-integer exponent gives a
complex result.
Example: power([1 2 3], 2)   % [1 4 9]
Differences from MATLAB: same limits as the .^ operator.

## mpower
Matrix power, the function form of a ^ b.
Syntax: C = mpower(A, B)
Evaluates a ^ b: repeated squaring for an integer exponent, eigendecomposition
for a non-integer exponent or a scalar base with a matrix exponent. An
ill-conditioned eigenvector basis produces a warning; results may be inaccurate.
Example: mpower([1 1; 0 1], 3)   % [1 3; 0 1]
Differences from MATLAB: same limits as the ^ operator.

## uminus
Unary minus, the function form of -a.
Syntax: B = uminus(A)
Evaluates -a with the operator's own rules.
Example: uminus([1 -2])   % [-1 2]
Differences from MATLAB: same limits as the unary - operator.

## uplus
Unary plus, the function form of +a.
Syntax: B = uplus(A)
Evaluates +a with the operator's own rules.
Example: uplus(5)   % 5
Differences from MATLAB: same limits as the unary + operator.

## eq
Equality test, the function form of a == b.
Syntax: TF = eq(A, B)
Returns a logical array, as the == operator does.
Example: eq([1 2 3], 2)   % [0 1 0]
Differences from MATLAB: same limits as the == operator.

## ne
Inequality test, the function form of a ~= b.
Syntax: TF = ne(A, B)
Returns a logical array, as the ~= operator does.
Example: ne([1 2 3], 2)   % [1 0 1]
Differences from MATLAB: same limits as the ~= operator.

## lt
Less-than test, the function form of a < b.
Syntax: TF = lt(A, B)
Returns a logical array, as the < operator does.
Example: lt([1 2 3], 2)   % [1 0 0]
Differences from MATLAB: same limits as the < operator.

## gt
Greater-than test, the function form of a > b.
Syntax: TF = gt(A, B)
Returns a logical array, as the > operator does.
Example: gt([1 2 3], 2)   % [0 0 1]
Differences from MATLAB: same limits as the > operator.

## le
Less-or-equal test, the function form of a <= b.
Syntax: TF = le(A, B)
Returns a logical array, as the <= operator does.
Example: le([1 2 3], 2)   % [1 1 0]
Differences from MATLAB: same limits as the <= operator.

## ge
Greater-or-equal test, the function form of a >= b.
Syntax: TF = ge(A, B)
Returns a logical array, as the >= operator does.
Example: ge([1 2 3], 2)   % [0 1 1]
Differences from MATLAB: same limits as the >= operator.

## nonzeros
Nonzero elements as a column.
Syntax: v = nonzeros(A)
Returns the nonzero elements of A in column order as a column vector,
0-by-1 when there are none.
Example: nonzeros([1 0 2; 0 3 0])   % [1; 3; 2]
Differences from MATLAB: none known.

## perms
All permutations.
Syntax: P = perms(v)
Returns every permutation of the elements of v, one per row, in reverse
lexicographic order of their positions. Char input gives char rows.
Example: perms([1 2 3])   % 6-by-3, first row [3 2 1]
Differences from MATLAB: the table is limited to 8388608 elements.

## gradient
Numerical gradient.
Syntax: FX = gradient(F)
Syntax: FX = gradient(F, h)
Syntax: [FX, FY] = gradient(F, hx, hy)
Central differences inside, one-sided differences at the edges. For a vector
F the derivative is along its length; for a matrix FX is along the rows
(across columns) and FY along the columns.
Example: gradient([1 4 9 16])   % [3 4 6 7]
Differences from MATLAB: scalar spacings only (no coordinate vectors); two
outputs at most.

## del2
Discrete Laplacian.
Syntax: L = del2(U)
Syntax: L = del2(U, h)
Syntax: L = del2(U, hx, hy)
Returns (d2U/dx2 + d2U/dy2)/4 with second differences inside and linear
extrapolation at the edges; a dimension with fewer than three points adds 0.
Example: del2([1 4 9 16 25])   % [0.5 0.5 0.5 0.5 0.5]
Differences from MATLAB: scalar spacings only; 2-D arrays only.

## unwrap
Correct phase angle jumps.
Syntax: Q = unwrap(P)
Syntax: Q = unwrap(P, tol)
Syntax: Q = unwrap(P, tol, dim)
Adds multiples of 2*pi where consecutive elements differ by at least tol
(default pi). Works along a row vector, otherwise down each column.
Example: unwrap([0 3 6.5 -3])   % [0 3 0.2168 3.2832]
Differences from MATLAB: real 2-D arrays only.

## hankel
Hankel matrix.
Syntax: H = hankel(c)
Syntax: H = hankel(c, r)
hankel(c) is square with c as its first column and zeros below the first
anti-diagonal. hankel(c, r) has first column c and last row r; when the last
element of c and the first of r differ, the column wins.
Example: hankel([1 2 3], [3 4 5])   % [1 2 3; 2 3 4; 3 4 5]
Differences from MATLAB: real input only; no warning is printed for the
anti-diagonal conflict.

## cplxpair
Sort complex numbers into conjugate pairs.
Syntax: Z = cplxpair(A)
Syntax: Z = cplxpair(A, tol)
Pairs complex conjugates in order of increasing real part, negative
imaginary part first, then the real elements in increasing order. Elements
with abs(imag(z)) <= tol*abs(z) (default tol = 100*eps) count as real.
Example: cplxpair([1+1i 2 1-1i])   % [1-1i 1+1i 2]
Differences from MATLAB: vectors only; no dim argument.

## orderfields
Order structure fields by name.
Syntax: S = orderfields(S1)
Returns S1 with its fields in ASCII order of their names.
Example: s.b = 1; s.a = 2; fieldnames(orderfields(s))   % {'a'; 'b'}
Differences from MATLAB: one input and one output only.

## narginchk
Check the number of input arguments.
Syntax: narginchk(minArgs, maxArgs)
Inside a function, stops with "Not enough input arguments." or "Too many
input arguments." when nargin is outside [minArgs, maxArgs].
Example: narginchk(1, 2)
Differences from MATLAB: none known.

## optimset
Create an optimization options structure.
Syntax: options = optimset('Name', value, ...)
Syntax: options = optimset(oldopts, 'Name', value, ...)
Returns a structure with one field per given option; a name given again
(case does not matter) replaces the earlier value.
Example: opts = optimset('TolX', 1e-8); opts.TolX   % 1.0000e-08
Differences from MATLAB: the structure holds only the options that were set,
not MATLAB's full list of optimization options, and names are not checked.

## getenv
Read an environment variable.
Syntax: value = getenv(name)
An unset variable returns an empty character array. Reads engine overrides first, then the inherited host environment.
Example: setenv('MATRUST_HELP_ENV', 'abc'); getenv('MATRUST_HELP_ENV')   % abc
Differences from MATLAB: setenv changes are local to an engine; they do not modify the host process environment.

## setenv
Set an environment variable for this engine.
Syntax: setenv(name, value)
Syntax: setenv(name)
One argument, or an empty value, sets the variable to empty. At most 1024 overrides and 1 MiB of names and values are retained.
Example: setenv('MATRUST_HELP_ENV', 'abc'); getenv('MATRUST_HELP_ENV')   % abc
Differences from MATLAB: Changes are local to the engine and its clones; other engines and the host process environment are unchanged.

## ispc
True on Windows.
Syntax: tf = ispc
Returns a logical scalar describing the host operating system.
Example: isscalar(ispc)   % 1
Differences from MATLAB: none known for this form.

## isunix
True on Unix systems, including macOS.
Syntax: tf = isunix
Returns a logical scalar describing the host operating system.
Example: isscalar(isunix)   % 1
Differences from MATLAB: none known for this form.

## ismac
True on macOS.
Syntax: tf = ismac
Returns a logical scalar describing the host operating system.
Example: isscalar(ismac)   % 1
Differences from MATLAB: none known for this form.

## computer
Identify the host computer architecture.
Syntax: c = computer
Syntax: arch = computer('arch')
Syntax: [c, maxsize, endian] = computer
The architecture names describe the supported 64-bit Windows, Linux and Intel macOS hosts. Endian is L or B. maxsize is the MATLAB-compatible theoretical limit; actual allocations retain MatRust limits.
Example: [c, n, e] = computer; ischar(c)   % 1
Differences from MATLAB: The reported theoretical maximum does not raise the engine allocation limit.

## version
Return the MatRust version.
Syntax: v = version
Syntax: r = version('-release')
Syntax: d = version('-date')
Identifies MatRust and its package version. The release is alpha; the build date is not recorded.
Example: version   % MatRust 0.1.2
Differences from MATLAB: Reports MatRust rather than MATLAB. The -date form returns not recorded.

## ver
Return MatRust product information.
Syntax: v = ver
Syntax: v = ver('matlab')
Syntax: v = ver('matrust')
Returns Name, Version, Release and Date fields. The matlab alias identifies MatRust, not an installed MATLAB product. Unknown products return an empty struct array with these fields. With no output, prints the information.
Example: v=ver('matlab'); v.Name   % MatRust
Differences from MATLAB: Reports MatRust package information; the Date field is empty because a build date is not recorded.

## verLessThan
Compare a product version numerically.
Syntax: tf = verLessThan(product, version)
Accepts one to three dot-separated unsigned version components. matlab compares the compatibility target, 9.0; matrust compares the package version. Unknown products report an error.
Example: verLessThan('matlab','9.1')   % 1
Differences from MATLAB: matlab denotes the 9.0 compatibility target, not a MATLAB installation; matrust denotes this application.

## matlabroot
Return the MatRust installation folder.
Syntax: folder = matlabroot
Returns the folder containing the running executable.
Example: ischar(matlabroot)   % 1
Differences from MATLAB: Returns the MatRust executable folder rather than a MATLAB installation folder.

## filesep
Return the host file separator.
Syntax: s = filesep
Returns a backslash on Windows and a forward slash on Unix.
Example: numel(filesep)   % 1
Differences from MATLAB: none known for this form.

## pathsep
Return the host search-path separator.
Syntax: s = pathsep
Returns a semicolon on Windows and a colon on Unix.
Example: numel(pathsep)   % 1
Differences from MATLAB: none known for this form.

## pwd
Return the current folder.
Syntax: folder = pwd
The folder belongs to this engine. File I/O, cd and desktop job submission use it. With no output, prints the folder.
Example: isdir(pwd)   % 1
Differences from MATLAB: The host process current directory is unchanged by engine folder changes.

## cd
Change the current folder.
Syntax: old = cd(folder)
Syntax: cd folder
Syntax: cd
Returns the previous folder when assigned. Sets file I/O and function lookup together; the desktop Files panel adopts changes after the job. With no argument, reports the current folder.
Example: old=cd(pwd); strcmp(old,pwd)   % 1
Differences from MATLAB: Changes the engine folder, not the host process current directory.

## dir
List file and directory information.
Syntax: entries = dir
Syntax: entries = dir(folderOrPattern)
Returns a column struct array with name, date, bytes, isdir and datenum fields. Directory listings include dot and dot-dot. Supports * and ? in the final path component; an unmatched pattern returns a 0-by-1 struct. With no output, prints names.
Example: d=tempname(pwd); mkdir(d); q=dir(d); n=numel(q); rmdir(d); n   % 2
Differences from MATLAB: Local paths only; parent-directory wildcards are rejected. Scans are limited to 10000 entries.

## ls
List file names.
Syntax: names = ls
Syntax: names = ls(folderOrPattern)
Returns a padded character matrix, one sorted name per row. With no output, prints the names. Uses the same wildcard and scan bounds as dir.
Example: d=tempname(pwd); mkdir(d); names=ls(d); rmdir(d); size(names)   % [2 2]
Differences from MATLAB: Always returns the Windows-style character matrix, including dot and dot-dot for directories.

## mkdir
Create a directory and any missing parents.
Syntax: status = mkdir(folder)
Syntax: [status, message, id] = mkdir(parent, child)
Returns logical success. An existing directory succeeds with message Directory already exists. Without outputs, failures raise errors and an existing directory emits a warning.
Example: d=tempname(pwd); ok=mkdir(d); rmdir(d); ok   % 1
Differences from MATLAB: Network paths are not supported. Host operating-system failure messages can differ.

## rmdir
Remove an empty directory or a directory tree.
Syntax: status = rmdir(folder)
Syntax: [status, message, id] = rmdir(folder, 's')
The s option removes contents recursively. Without outputs, failures raise errors. Recursive removal checks cancellation and is bounded to 64 levels and 10000 entries; cancellation or failure may leave a partly removed tree.
Example: d=tempname(pwd); mkdir(d); rmdir(d)   % no output
Differences from MATLAB: Local paths only; recursive operations have explicit bounds and cooperative cancellation.

## fileread
Read a UTF-8 text file into a character row.
Syntax: text = fileread(filename)
Reads in bounded chunks, checks cancellation and keeps every byte as read, so CRLF line endings stay (as in MATLAB). The result is limited to 65536 UTF-16 units.
Example: d=tempname(pwd); mkdir(d); f=fullfile(d,'a.txt'); fid=fopen(f,'w'); fprintf(fid,'abc'); fclose(fid); text=fileread(f); rmdir(d,'s'); text   % abc
Differences from MATLAB: Only UTF-8 input is supported; non-UTF-8 files are rejected. Character arrays have the engine size limit.

## tempdir
Return the host temporary folder.
Syntax: folder = tempdir
The returned character row ends with a file separator.
Example: isdir(tempdir)   % 1
Differences from MATLAB: none known for this form.

## tempname
Generate a unique temporary path without creating a file.
Syntax: name = tempname
Syntax: name = tempname(folder)
Uses the supplied folder or the host temporary folder. Names include process, time and sequence components. A name is not a reservation: another process can still create it.
Example: a=tempname(pwd); b=tempname(pwd); strcmp(a,b)   % 0
Differences from MATLAB: The generated name format differs from MATLAB.

## clock
Return the current local date and time.
Syntax: v = clock
Returns [year month day hour minute seconds], with fractional seconds.
Example: size(clock)   % [1 6]
Differences from MATLAB: none known for this form on supported Windows, Linux and macOS hosts.

## cputime
Return total process CPU time in seconds.
Syntax: t = cputime
Includes user and kernel CPU time from all threads in this process. It does not measure elapsed wall time.
Example: class(cputime)   % double
Differences from MATLAB: Measures the MatRust process, including other engines and desktop work in the same process.

## date
Return the current local date.
Syntax: s = date
Returns an eleven-character dd-mmm-yyyy row with an English month abbreviation.
Example: size(date)   % [1 11]
Differences from MATLAB: none known for this form.

## now
Return the current local time as a serial date number.
Syntax: n = now
Uses the same serial day numbering as datenum; the fraction represents time within the day.
Example: class(now)   % double
Differences from MATLAB: none known for this form.

## pause
Wait cooperatively or control timed pauses.
Syntax: pause(seconds)
Syntax: pause('on')
Syntax: pause('off')
Syntax: state = pause('query')
Zero and negative durations return immediately. Timed pauses check cancellation at least every 10 ms between sleeps. The on/off state belongs to the engine.
Example: pause(0); pause('query')   % on
Differences from MATLAB: No-argument key waiting is not supported yet and gives an explicit error.

## timeit
Estimate the elapsed time of a zero-input function handle.
Syntax: seconds = timeit(f)
Syntax: seconds = timeit(f, outputCount)
Warms up once and returns the median of five batches. Each batch performs at most 64 calls or 25 ms of work; a single call can take longer. Output count defaults to one. Ordinary execution budgets and cancellation apply. The function may run many times, so account for side effects.
Example: class(timeit(@() ones(10)))   % double (a fast-call warning may also appear)
Differences from MATLAB: Uses a bounded sampling algorithm rather than MATLAB calibration; interpreter overhead is included. Explicit output counts are limited to 0 through 16.

## isdir
True if a local directory exists.
Syntax: tf = isdir(path)
Resolves relative paths against the engine current folder and returns a logical scalar.
Example: isdir(pwd)   % 1
Differences from MATLAB: Network paths are not supported.

## copyfile
Copy local files or directory trees.
Syntax: status = copyfile(source, destination)
Syntax: [status, message, id] = copyfile(source, destination, 'f')
Destination defaults to the current folder. A file copied to a directory keeps its basename; a directory source copies its contents into the destination. Wildcard sources create a missing destination folder. Supports final-component * and ? patterns. The f option permits replacing read-only files. Without outputs, failure raises an error.
Example: d=tempname(pwd); mkdir(d); f=fullfile(d,'a'); fclose(fopen(f,'w')); ok=copyfile(f,fullfile(d,'b')); rmdir(d,'s'); ok   % 1
Differences from MATLAB: Local paths only; symbolic links are rejected. Copies are bounded to 256 MiB, 10000 entries and 64 levels. Each file is staged before replacement; cancellation or failure can leave a partially copied directory tree.

## movefile
Move local files or directory trees.
Syntax: status = movefile(source, destination)
Syntax: [status, message, id] = movefile(source, destination, 'f')
Uses an operating-system rename on the same filesystem. Across filesystems, copies using copyfile rules and removes the source after the copy completes. Supports final-component wildcard patterns. Without outputs, failure raises an error.
Example: d=tempname(pwd); mkdir(d); f=fullfile(d,'a'); fclose(fopen(f,'w')); ok=movefile(f,fullfile(d,'b')); rmdir(d,'s'); ok   % 1
Differences from MATLAB: Across filesystems, the copyfile bounds apply and interruption can leave both copies. Symbolic links are rejected.

## delete
Delete local files.
Syntax: delete(filename)
Syntax: delete(pattern1, pattern2, ...)
Supports * and ? in the final path component and at most 64 patterns. A missing file emits a warning. Use rmdir to remove directories.
Example: d=tempname(pwd); mkdir(d); f=fullfile(d,'a'); fclose(fopen(f,'w')); delete(f); n=exist(f,'file'); rmdir(d); n   % 0
Differences from MATLAB: Local file deletion only; graphics-object deletion is not supported by this function.

## movmax
Moving maximum.
Syntax: y = movmax(A, k[, dim][, nanflag][, 'Endpoints', e])
A scalar k centers each window, with one extra sample behind for even k. [kb kf] specifies the samples before and after. The default dimension is the first nonsingleton dimension. The default NaN policy is omitnan; omitnan and includenan are accepted. Endpoints may be shrink (default), discard, fill, or a scalar. The input class is preserved.
Example: movmax([1 5 3], 2)   % [1 5 5]
Differences from MATLAB: Complex data, SamplePoints and dimensions above 2 are not supported yet.

## movmin
Moving minimum.
Syntax: y = movmin(A, k[, dim][, nanflag][, 'Endpoints', e])
A scalar k centers each window, with one extra sample behind for even k. [kb kf] specifies the samples before and after. The default dimension is the first nonsingleton dimension. The default NaN policy is omitnan; omitnan and includenan are accepted. Endpoints may be shrink (default), discard, fill, or a scalar. The input class is preserved.
Example: movmin([1 5 3], 2)   % [1 1 3]
Differences from MATLAB: Complex data, SamplePoints and dimensions above 2 are not supported yet.

## movmedian
Moving median.
Syntax: y = movmedian(A, k[, dim][, nanflag][, 'Endpoints', e])
A scalar k centers each window, with one extra sample behind for even k. [kb kf] specifies the samples before and after. The default dimension is the first nonsingleton dimension. The default NaN policy is includenan; omitnan and includenan are accepted. Endpoints may be shrink (default), discard, fill, or a scalar. The input class is preserved.
Example: movmedian([1 5 3], 3)   % [3 3 4]
Differences from MATLAB: Complex data, SamplePoints and dimensions above 2 are not supported yet.

## movstd
Moving standard deviation.
Syntax: y = movstd(A, k[, 0|1][, dim][, nanflag][, 'Endpoints', e])
Accepts real double, single and logical data; single results retain their class.
A scalar k centers each window, with one extra sample behind for even k. [kb kf] specifies the samples before and after. The default dimension is the first nonsingleton dimension. The default NaN policy is includenan; omitnan and includenan are accepted. Endpoints may be shrink (default), discard, fill, or a scalar. Standard deviation and variance use sample normalization (0); 1 selects population normalization.
Example: movstd([1 3 5], 3)   % [1.4142 2.0000 1.4142]
Differences from MATLAB: Complex data, SamplePoints and dimensions above 2 are not supported yet.

## movvar
Moving variance.
Syntax: y = movvar(A, k[, 0|1][, dim][, nanflag][, 'Endpoints', e])
Accepts real double, single and logical data; single results retain their class.
A scalar k centers each window, with one extra sample behind for even k. [kb kf] specifies the samples before and after. The default dimension is the first nonsingleton dimension. The default NaN policy is includenan; omitnan and includenan are accepted. Endpoints may be shrink (default), discard, fill, or a scalar. Standard deviation and variance use sample normalization (0); 1 selects population normalization.
Example: movvar([1 3 5], 3)   % [2 4 2]
Differences from MATLAB: Complex data, SamplePoints and dimensions above 2 are not supported yet.

## repelem
Repeat individual array elements.
Syntax: B = repelem(A, R) or repelem(A, R1, R2)
The two-input form repeats each vector element. The three-input form repeats rows and columns; each repetition factor can be a scalar or a vector matching its dimension. Numeric classes, logical, character and cell values retain their class.
Example: repelem([1 2 3], [1 0 2])   % [1 3 3]
Differences from MATLAB: More than two dimensions and handle arrays are not supported yet.

## isequaln
Test equality, treating corresponding NaNs as equal.
Syntax: tf = isequaln(A, B, ...)
Shapes and contents must agree. Cells and structures are compared recursively; numeric classes may differ.
Example: isequaln({NaN, 1}, {NaN, 1})   % 1
Differences from MATLAB: No N-D arrays.

## isdiag
Test whether all entries off the main diagonal are zero.
Syntax: tf = isdiag(A)
Accepts numeric and logical matrices, including rectangular and empty matrices.
Example: isdiag([1 0; 0 2])   % 1
Differences from MATLAB: Character and cell inputs are rejected, matching MATLAB.

## istril
Test whether all entries above the main diagonal are zero.
Syntax: tf = istril(A)
Accepts numeric and logical matrices, including rectangular and empty matrices.
Example: istril([1 0; 2 3])   % 1
Differences from MATLAB: Character and cell inputs are rejected, matching MATLAB.

## istriu
Test whether all entries below the main diagonal are zero.
Syntax: tf = istriu(A)
Accepts numeric and logical matrices, including rectangular and empty matrices.
Example: istriu([1 2; 0 3])   % 1
Differences from MATLAB: Character and cell inputs are rejected, matching MATLAB.

## isbanded
Test whether nonzero entries lie within a diagonal band.
Syntax: tf = isbanded(A, lower, upper)
The lower and upper bandwidths must be nonnegative integer scalars. Empty matrices satisfy the test.
Example: isbanded([1 2 0; 3 4 5; 0 6 7], 1, 1)   % 1
Differences from MATLAB: No N-D arrays.

## issymmetric
Test equality with the transpose.
Syntax: tf = issymmetric(A[, 'skew'|'nonskew'])
The default nonskew test compares A with its transpose. The skew test compares A with the negative of that transpose. Nonsquare matrices return false; comparisons are exact.
Example: issymmetric([0 2; -2 0], 'skew')   % 1
Differences from MATLAB: A numeric tolerance is rejected, matching MATLAB. No N-D arrays.

## ishermitian
Test equality with the conjugate transpose.
Syntax: tf = ishermitian(A[, 'skew'|'nonskew'])
The default nonskew test compares A with its conjugate transpose. The skew test compares A with the negative of that transpose. Nonsquare matrices return false; comparisons are exact.
Example: ishermitian([1 2+1i; 2-1i 3])   % 1
Differences from MATLAB: A numeric tolerance is rejected, matching MATLAB. No N-D arrays.

## bandwidth
Find the lower and upper matrix bandwidth.
Syntax: [lower, upper] = bandwidth(A); b = bandwidth(A, 'lower'|'upper')
Bandwidth is the greatest distance from the main diagonal to a nonzero entry. One output without an option returns the lower bandwidth. Zero and empty matrices have bandwidth zero.
Example: [lo, up] = bandwidth([1 2; 0 3]); [lo up]   % [0 1]
Differences from MATLAB: No N-D arrays.

## uniquetol
Find sorted unique values within a scaled tolerance.
Syntax: [C, ia, ic] = uniquetol(A[, tol][, 'ByRows', true][, 'DataScale', scale])
Real double and single inputs are accepted. The default tolerance is 1e-12 for double and 1e-6 for single. Values match when their absolute difference is within tol times the largest absolute input value; ByRows uses a separate scale for each column. DataScale overrides it. NaNs remain distinct. OutputAllIndices returns a cell of matching input indices instead of ia.
Example: uniquetol([1 1.01 2], 0.01)   % [1 2]
Differences from MATLAB: N-D input is not supported.

## ismembertol
Test membership within a scaled tolerance.
Syntax: [tf, loc] = ismembertol(A, B[, tol][, 'ByRows', true][, 'DataScale', scale])
Returns a logical array and the first matching B index (zero for no match). Defaults are 1e-12 for double and 1e-6 when either input is single. Scaling uses both inputs, per column with ByRows. NaNs never match. OutputAllIndices returns all matching B indices in a cell array.
Example: [tf, loc] = ismembertol([2 7], [1 2 3]); loc   % [2 0]
Differences from MATLAB: N-D input is not supported.

## histcounts2
Count pairs of observations in two-dimensional bins.
Syntax: [N, xe, ye, binX, binY] = histcounts2(X, Y[, n | [nx ny] | xedges, yedges][, 'BinWidth', [wx wy]][, 'Normalization', mode])
X and Y must have the same size. N has one row per X bin and one column per Y bin. Bins include their left edge; the final bin includes both edges. NaN pairs are omitted. Normalizations: count, probability, countdensity, pdf, cumcount and cdf.
Example: histcounts2([0.5 1.5], [0.5 1.5], [0 1 2], [0 1 2])   % [1 0; 0 1]
Differences from MATLAB: BinMethod and bin-limit options are not supported yet. Automatic edge generation is limited to 65,536 bins per axis; very large floating-point ranges require explicit edges.

## bitget
Get a bit from each integer value.
Syntax: bitget(A, BIT); bitget(A, BIT, ASSUMEDTYPE)
Bits are numbered from 1 at the least significant bit. Equal-size arrays or scalar inputs are accepted. Integer inputs retain their class and all 64 bits. Double inputs must be integral and default to an assumed uint64 type; the optional assumed type selects an integer bit width.
Example: bitget(5, 1:4)   % [1 0 1 0]
Differences from MATLAB: none known.

## bitset
Set or clear a bit in each integer value.
Syntax: bitset(A, BIT); bitset(A, BIT, V); bitset(A, BIT, V, ASSUMEDTYPE)
V defaults to 1; zero clears a bit and a nonzero value sets it. Equal-size arrays or scalar inputs are accepted. Integer output keeps the input class, including the sign bit of signed integers. Double inputs default to an assumed uint64 type.
Example: bitset([1 2 4], 1)   % [1 3 5]
Differences from MATLAB: none known.

## swapbytes
Reverse the bytes of each numeric element.
Syntax: swapbytes(A)
Preserves the numeric class and shape. Supports double, single and all integer classes. One-byte integers are unchanged. Real numeric input is required.
Example: swapbytes(uint16(1))   % uint16(256)
Differences from MATLAB: none known.

## num2hex
Show floating-point values as hexadecimal bit patterns.
Syntax: num2hex(A)
Returns one character row per element, in column-major order: 16 hexadecimal digits for double and 8 for single. Integer and complex inputs are rejected.
Example: num2hex(1)   % '3ff0000000000000'
Differences from MATLAB: Character output is limited to 65536 UTF-16 units.

## hex2num
Convert hexadecimal bit patterns to floating-point values.
Syntax: hex2num(S)
Accepts a character matrix or a cell array of character vectors and returns a column of doubles; empty input returns []. Blanks and short rows are padded with zeros. Both upper- and lower-case hexadecimal digits are accepted.
Example: hex2num('3ff')   % 1
Differences from MATLAB: Character inputs are limited to 65536 UTF-16 units.

## flipdim
Reverse an array along a dimension.
Syntax: flipdim(A, DIM)
Reverses rows for DIM=1 or columns for DIM=2, preserving the class. Dimensions greater than 2 leave a 2-D array unchanged.
Example: flipdim([1 2 3], 2)   % [3 2 1]
Differences from MATLAB: N-D arrays are not supported yet.

## shiftdim
Shift array dimensions or remove leading singleton dimensions.
Syntax: shiftdim(A); shiftdim(A, N); [B, N] = shiftdim(A)
Without N, a row becomes a column and N reports the number of removed leading singleton dimensions. Positive N rotates dimensions; negative N adds leading singleton dimensions. Results must fit in two dimensions.
Example: shiftdim([1 2; 3 4], 1)   % [1 3; 2 4]
Differences from MATLAB: N-D inputs and results are not supported yet.

## permute
Reorder array dimensions.
Syntax: permute(A, ORDER)
ORDER must be a permutation containing at least two indices. [1 2] preserves the array; [2 1] transposes without conjugation. Trailing singleton dimensions are accepted when the result remains 2-D.
Example: permute([1 2; 3 4], [2 1])   % [1 3; 2 4]
Differences from MATLAB: N-D inputs and results are not supported yet.

## ipermute
Undo a permutation of array dimensions.
Syntax: ipermute(A, ORDER)
Applies the inverse of ORDER, preserving the class and complex values. Trailing singleton dimensions are accepted when the result remains 2-D.
Example: ipermute([1 3; 2 4], [2 1])   % [1 2; 3 4]
Differences from MATLAB: N-D inputs and results are not supported yet.

## filter2
Apply a two-dimensional finite impulse response filter.
Syntax: filter2(H, X); filter2(H, X, SHAPE)
Computes conv2(X, rot90(H,2), SHAPE). Logical and integer inputs are converted to double first. SHAPE is 'same' by default; 'full' and 'valid' are also accepted. The kernel is rotated without conjugation.
Example: filter2([1 2 1], [1 2 3 4])   % [4 8 12 11]
Differences from MATLAB: Uses the existing bounded double/complex-double conv2 implementation.

## interpft
Interpolate periodic data using Fourier coefficients.
Syntax: interpft(X, N); interpft(X, N, DIM)
Resamples each vector along the first nonsingleton dimension, or DIM=1 or 2. Pads Fourier coefficients and splits the Nyquist term when needed. Complex input is retained. Smaller N samples the interpolated periodic signal; N=0 returns an empty array.
Example: interpft([1 2 3 4], 2)   % [1 3]
Differences from MATLAB: Transforms are limited to 1048576 points and two dimensions; FFT rounding may differ from MATLAB.

## interp1q
Quick piecewise-linear interpolation of column data.
Syntax: interp1q(X, Y, XI)
X must be a strictly increasing column with at least two points. Each column of Y has one sample per X. XI is a column of query points; queries outside the domain and NaN queries return NaN.
Example: interp1q([1;2;3], [10;20;30], [1.5;2.5])   % [15;25]
Differences from MATLAB: This implementation accepts real data only.

## rectint
Calculate areas of intersection between rectangles.
Syntax: rectint(A, B)
Each row contains [x y width height]. The result has one row per rectangle in A and one column per rectangle in B. Rectangles touching only at an edge have zero intersection area.
Example: rectint([0 0 2 2], [1 1 2 2])   % 1
Differences from MATLAB: none known.

## polyarea
Calculate polygon areas.
Syntax: polyarea(X, Y); polyarea(X, Y, DIM)
X and Y must have the same size. Computes the absolute shoelace area along the first nonsingleton dimension, or DIM=1 or 2. The last vertex is connected to the first.
Example: polyarea([0 1 0], [0 0 1])   % 0.5000
Differences from MATLAB: N-D arrays are not supported yet; DIM greater than 2 returns zeros(size(X)).

## inpolygon
Test whether points lie inside or on polygon boundaries.
Syntax: IN = inpolygon(X, Y, XV, YV); [IN, ON] = inpolygon(X, Y, XV, YV)
Returns logical arrays. Boundary points count as inside; ON identifies those points. Scalar point coordinates expand. NaN-separated polygon loops use nonzero winding: reverse an inner loop to make a hole. NaN query points are outside.
Example: inpolygon([0.5 2 1], [0.5 2 0.5], [0 1 1 0], [0 0 1 1])   % [true false true]
Differences from MATLAB: Point-edge work is bounded to 2^32 pairs.

## peaks
Evaluate the peaks test surface.
Syntax: peaks; peaks(N); peaks(V); peaks(X, Y); [X, Y, Z] = peaks(N)
The default grid is 49-by-49 over [-3,3]. N selects a square grid; V supplies grid coordinates. With two inputs, evaluates corresponding coordinates directly. Three outputs return the grid coordinates and heights.
Example: size(peaks)   % [49 49]
Differences from MATLAB: Returns numeric arrays; the no-output form does not create a surface plot.

## hsv2rgb
Convert hue, saturation and value to red, green and blue.
Syntax: hsv2rgb(MAP); [R, G, B] = hsv2rgb(MAP); [R, G, B] = hsv2rgb(H, S, V)
A colormap is a double M-by-3 matrix. Three outputs return separate color channels. Three channel inputs must have equal sizes. A colormap value outside [0,1] is an error, as in MATLAB.
Example: hsv2rgb([0 1 1])   % [1 0 0]
Differences from MATLAB: N-D color images and the three-input single-output form are not supported yet.

## rgb2hsv
Convert red, green and blue to hue, saturation and value.
Syntax: rgb2hsv(MAP); [H, S, V] = rgb2hsv(MAP); [H, S, V] = rgb2hsv(R, G, B)
A colormap is a double M-by-3 matrix. Hue and saturation are zero for black; hue is zero for gray. Three outputs return separate channels. MATLAB rejects uint8 colormaps.
Example: rgb2hsv([1 0 0])   % [0 1 1]
Differences from MATLAB: N-D color images and the three-input single-output form are not supported yet.

## atan2d
Four-quadrant inverse tangent in degrees.
Syntax: atan2d(Y, X)
Inputs must have equal sizes or one must be scalar. Accepts real double or single input; a single input produces a single result. Both zero coordinates produce zero, including a negative-zero X.
Example: atan2d([1 -1 0], [0 0 -1])   % [90 -90 180]
Differences from MATLAB: none known.

## full
Return a full array.
Syntax: full(A)
Already-full numeric, logical and character arrays are returned with the same shape, class and shared storage. Cells and structs are rejected.
Example: full([1 2 3])   % [1 2 3]
Differences from MATLAB: Sparse arrays are not implemented, so sparse-to-full conversion cannot yet be requested.

## evalc
Evaluate code and capture its console text.
Syntax: text = evalc(code); [text, value] = evalc(expression)
Example: strcmp(evalc('disp(5)'), sprintf('     5\n\n'))   % true
Differences from MATLAB: At most one expression result in addition to captured text.

## evalin
Evaluate code in the base or caller workspace.
Syntax: value = evalin('base'|'caller', code [, catchCode])
Example: a=7; evalin('base','a+1')   % 8
Differences from MATLAB: Function definitions inside evaluated text are rejected.

## assignin
Assign a variable in the base or caller workspace.
Syntax: assignin('base'|'caller', name, value)
Example: assignin('base','answer',42); answer   % 42
Differences from MATLAB: Variable names use the engine's ASCII identifier rules.

## global
Declare variables shared by function workspaces.
Syntax: global name1 name2
New globals are empty; clear global name removes a shared variable.
Example: global shared; shared=3; shared   % 3
Differences from MATLAB: Global storage is bounded by the engine's workspace limits.

## persistent
Retain a function variable between calls.
Syntax: persistent name1 name2
Declarations are allowed only inside functions. clear functionName resets saved values.
Example: a=tick(); b=tick(); [a b], function y=tick(), persistent n; if isempty(n), n=0; end; n=n+1; y=n; end   % [1 2]
Differences from MATLAB: Persistent storage is bounded by the engine's workspace limits.

## inputname
Return the caller's variable name for a function input.
Syntax: name = inputname(index)
Expressions have an empty input name, and so do calls made through cellfun, arrayfun, feval or a function handle.
Example: v=3; name=whichinput(v); name, function y=whichinput(a), y=inputname(1); end   % v
Differences from MATLAB: No active function produces an error, as in MATLAB.

## nargoutchk
Check the number of requested function outputs.
Syntax: nargoutchk(minimum, maximum)
Syntax: message = nargoutchk(minimum, maximum, count [, 'string'|'struct'])
Example: nargoutchk(1,2,3)   % Too many output arguments.
Differences from MATLAB: none known for these forms.

## nargchk
Return a diagnostic for an input count outside a range.
Syntax: message = nargchk(minimum, maximum, count [, 'string'|'struct'])
Example: warning('off','all'); nargchk(1,2,3)   % Too many input arguments.
Differences from MATLAB: Emits MATLAB's deprecation warning.

## validateattributes
Validate an array's class, shape and values.
Syntax: validateattributes(value, classes, attributes [, functionName, argumentName, position])
Unknown attribute names warn and are skipped, as in MATLAB. A failed size check has the identifier MATLAB:incorrectSize, or MATLAB:functionName:incorrectSize when a function name is given.
Example: validateattributes([1 2],{'double'},{'row','positive'});   % no output
Differences from MATLAB: Arrays are two-dimensional, so 3d always passes and ndims passes only for 2. Messages for numel, nrows, ncols, ndims, even, odd, binary and diag are not checked against MATLAB yet.

## lasterr
Read or replace the previous error message and identifier.
Syntax: [message, identifier] = lasterr([newMessage [, newIdentifier]])
Example: lasterr('old','pkg:id'); x=lasterr('new')   % old
Differences from MATLAB: Stored messages omit MATLAB script and line-number prefixes.

## lastwarn
Read or replace the previous warning message and identifier.
Syntax: [message, identifier] = lastwarn([newMessage [, newIdentifier]])
Example: warning('off','all'); warning('pkg:id','notice'); lastwarn   % notice
Differences from MATLAB: Console warnings omit MATLAB desktop backtrace links.

## MException
Create an exception with an identifier, message, cause list and stack.
Syntax: exception = MException(identifier, format, arguments...)
A caught error's stack lists the function frames where it was raised, innermost first, ending with 'submitted source'; an error raised at top level has an empty 0x1 stack. The exception is not a structure: isstruct is false and its properties (identifier, message, stack, cause) are read-only.
Example: e=MException('pkg:id','Value %d',5); e.message   % Value 5
Differences from MATLAB: Uses tagged structure storage; stack file names and line numbers are not tracked.

## throw
Raise an MException.
Syntax: throw(exception)
Example: try, throw(MException('pkg:id','failed')); catch e, e.message, end   % failed
Differences from MATLAB: Stack line numbers are not tracked.

## throwAsCaller
Raise an MException with the caller as its stack origin.
Syntax: throwAsCaller(exception)
Example: try, throwAsCaller(MException('pkg:id','failed')); catch e, e.message, end   % failed
Differences from MATLAB: Stack line numbers are not tracked.

## onCleanup
Run a callback when its cleanup object leaves all workspaces.
Syntax: object = onCleanup(task)
Example: c=onCleanup(@() disp('done')); clear c;   % done
Differences from MATLAB: Uses tagged structure storage; callbacks run at interpreter statement and function boundaries.

## cell2struct
Convert cell contents to fields along a selected dimension.
Syntax: structure = cell2struct(cells, fieldNames, dimension)
Example: s=cell2struct({3;'hi'},{'n','text'},1); s.n   % 3
Differences from MATLAB: Dimensions 1 and 2 only.

## struct2cell
Convert structure fields to a cell array.
Syntax: cells = struct2cell(structure)
Example: c=struct2cell(struct('a',3,'b',4)); c{2}   % 4
Differences from MATLAB: Results requiring a third dimension are rejected.

## substruct
Construct a structure describing subscript operations.
Syntax: subscripts = substruct(type, subs, ...)
Example: s=substruct('.','field'); s.type   % .
Differences from MATLAB: none known for these forms.

## celldisp
Print cell contents with indexed labels, recursively.
Syntax: celldisp(cells [, name])
Example: celldisp({})   % no output
Differences from MATLAB: Nesting is bounded to 32 levels.

## mat2cell
Partition a matrix into cells of specified row and column sizes.
Syntax: cells = mat2cell(matrix, rowSizes [, columnSizes])
Example: c=mat2cell([1 2 3],1,[1 2]); c{2}   % [2 3]
Differences from MATLAB: Two-dimensional arrays only.

## sscanf
Scan formatted values from character text.
Syntax: [values, count, message, nextIndex] = sscanf(text, format [, size])
count is the number of successful conversions, so sscanf('abc','%s') counts 1.
Example: sscanf('3 4 5','%d')'   % [3 4 5]
Differences from MATLAB: Conversions other than d, i, u, f, e, g, s, c, x and o are rejected.

## strread
Read formatted text into separate output columns.
Syntax: [a, b, ...] = strread(text [, format, option, value, ...])
Example: [a,b]=strread('1 2 3 4','%d %d'); [a b]   % [1 2; 3 4]
Differences from MATLAB: Uses the bounded textscan parser and its supported options.

## textread
Read formatted file text into separate output columns.
Syntax: [a, b, ...] = textread(filename [, format, option, value, ...])
Example: try, textread('no_such_file_zz.txt','%d'); catch e, e.message, end   % File not found.
Differences from MATLAB: Uses the bounded textscan parser and its supported options.

## str2mat
Pad character rows to form a character matrix.
Syntax: chars = str2mat(row1, row2, ...)
Example: size(str2mat('a','bcd'))   % [2 3]
Differences from MATLAB: none known for these forms.

## strmatch
Find rows beginning with a prefix, optionally requiring an exact match.
Syntax: indices = strmatch(prefix, charMatrixOrCells [, 'exact'])
Example: strmatch('ab',{'abc','xab','ab'})'   % [1 3]
Differences from MATLAB: none known for these forms.

## findstr
Find overlapping occurrences of the shorter string in the longer one.
Syntax: indices = findstr(a, b)
Example: findstr('aaa','aa')   % [1 2]
Differences from MATLAB: Character inputs only.

## regexptranslate
Escape regular-expression metacharacters or translate wildcards.
Syntax: result = regexptranslate('escape'|'wildcard', textOrCells)
Example: regexptranslate('wildcard','*.m')   % .*\.m
Differences from MATLAB: Only escape and wildcard modes are supported.

## isstr
Test whether a value is a character array.
Syntax: tf = isstr(value)
Example: isstr('abc')   % true
Differences from MATLAB: none known for these forms.

## setstr
Convert numeric character codes to a character array.
Syntax: chars = setstr(codes)
Example: setstr([72 105])   % Hi
Differences from MATLAB: Cell input is rejected, as in MATLAB.

## symvar
Extract sorted variable names from an expression string.
Syntax: names = symvar(expression)
Example: symvar('x+y*z')   % {'x'; 'y'; 'z'}
Differences from MATLAB: A lexical scan identifies names; no symbolic engine is used.

## vectorize
Add dots to multiplication, division and power operators in text.
Syntax: text = vectorize(expressionOrHandle)
Example: vectorize('x*y^2/z')   % x.*y.^2./z
Differences from MATLAB: none known for these forms.

## who
List workspace variable names, optionally filtered by wildcards.
Syntax: names = who(pattern, ...); who pattern
Syntax: names = who('-file', filename [, pattern, ...])
Example: zzw1=1; zzw2=2; who('zzw*')   % {'zzw1'; 'zzw2'}
Differences from MATLAB: Command output uses one row of names.

## rehash
Refresh negative function-file lookup results.
Syntax: rehash
Example: rehash   % no output
Differences from MATLAB: No toolbox cache options.

## addCause
Append an exception to another exception's cause list.
Syntax: exception = addCause(exception, cause)
Example: e=addCause(MException('a:b','outer'),MException('c:d','inner')); e.cause{1}.message   % inner
Differences from MATLAB: Uses tagged structure storage.

## getReport
Return an exception's message followed by a newline.
Syntax: text = getReport(exception)
Example: strtrim(getReport(MException('a:b','text')))   % text
Differences from MATLAB: Extended reports and hyperlinks are not implemented.

## properties
List the supported MException properties.
Syntax: names = properties('MException')
Example: properties('MException')   % {'identifier'; 'message'; 'cause'; 'stack'}
Differences from MATLAB: Only the MException class-name form is implemented.

## subsref
Apply a substruct description to a value.
Syntax: value = subsref(value, subscripts)
Example: s=struct('a',[10 20]); subsref(s,substruct('.','a','()',{2}))   % 20
Differences from MATLAB: Numeric and colon subscripts on two-dimensional arrays only.

## rref
Reduced row echelon form and pivot columns.
Syntax: R = rref(A); [R,jb] = rref(A,tol)
Example: rref([1 2; 2 4])   % [1 2; 0 0]
Differences from MATLAB: Real double matrices; finite nonnegative scalar tolerance.

## hess
Hessenberg reduction by an orthogonal or unitary similarity.
Syntax: H = hess(A); [Q,H] = hess(A)
Example: hess([1 2; 3 4])   % [1 2; 3 4]
Differences from MATLAB: Square double or complex matrices; the two-matrix generalized form is not implemented.

## schur
Real or complex Schur decomposition.
Syntax: T = schur(A); [U,T] = schur(A,'complex'); [U,T] = schur(A,'real')
Example: schur([1 2; 0 3])   % [1 2; 0 3]
Differences from MATLAB: Dense double or complex matrices; no single-precision output.

## ldl
Symmetric indefinite factorization with rook pivoting.
Syntax: L = ldl(A); [L,D] = ldl(A); [L,D,P] = ldl(A); [U,D,P] = ldl(A,'upper')
Example: [L,D] = ldl([4 2; 2 3]); D   % [4 0; 0 2]
Differences from MATLAB: Real double matrices; lower triangle by default. The vector permutation option is not implemented.

## linsolve
Solve linear systems using matrix properties when supplied.
Syntax: X = linsolve(A,B); [X,R] = linsolve(A,B,opts)
Example: linsolve([2 0; 1 3], [4; 5])   % [2; 1]
Differences from MATLAB: Real double inputs. LT, UT and TRANSA select triangular and transposed solves for a square A; a rectangular A always gets the pivoted-QR least-squares solution. SYM, POSDEF and RECT use the general solver.

## lscov
Least squares with optional weights or covariance.
Syntax: X = lscov(A,B); [X,SE,MSE,S] = lscov(A,B,V,algorithm)
Example: lscov([1; 1], [2; 4])   % 3
Differences from MATLAB: Real double inputs; full covariance must be positive definite. The fourth output requires one response column; uncertainty outputs require a nonempty design matrix.

## lsqnonneg
Nonnegative linear least squares by an active-set iteration.
Syntax: X = lsqnonneg(C,d); [X,resnorm,residual,exitflag,output,lambda] = lsqnonneg(C,d,options)
Example: lsqnonneg(eye(2), [2; -1])   % [2; 0]
Differences from MATLAB: Real double inputs and one response column; options TolX and MaxIter are supported.

## rcond
Reciprocal of the matrix one-norm condition number.
Syntax: R = rcond(A)
Example: rcond(eye(2))   % 1
Differences from MATLAB: Computes an exact inverse one-norm using LU vector solves instead of a condition estimator; does not form an inverse matrix.

## condest
Matrix one-norm condition number and a normalized sensitive direction.
Syntax: C = condest(A); [C,v] = condest(A,t)
Example: condest(eye(2))   % 1
Differences from MATLAB: Uses exact LU vector solves for dense real double matrices; t is validated but does not change the calculation.

## normest
Estimate the matrix two-norm by power iteration.
Syntax: N = normest(A); [N,count] = normest(A,tol)
Example: normest([3 4])   % 5
Differences from MATLAB: Real double matrices and a positive finite tolerance.

## subspace
Largest principal angle between two column spaces.
Syntax: theta = subspace(A,B)
Example: subspace([1; 0], [2; 0])   % 0
Differences from MATLAB: Real double matrices; bases use the existing SVD orth routine.

## sylvester
Solve the Sylvester equation A*X + X*B = C.
Syntax: X = sylvester(A,B,C)
Example: sylvester(2,3,10)   % 2
Differences from MATLAB: Real double input; singular transformed equations are rejected.

## polyvalm
Evaluate a polynomial at a square matrix.
Syntax: Y = polyvalm(p,A)
Example: polyvalm([1 2 3], [1 2; 3 4])   % [12 14; 21 33]
Differences from MATLAB: Real double input; degree is bounded at 4096.

## balance
Balance matrix row and column norms by a similarity transformation.
Syntax: B = balance(A); [T,B] = balance(A); [scale,p,B] = balance(A,'noperm')
Example: balance([1 100; 0.01 1])   % [1 0.78125; 1.28 1]
Differences from MATLAB: Real double input; permutation and power-of-two scaling with a bounded iteration count.

## planerot
Plane rotation that annihilates the second entry.
Syntax: [G,y] = planerot(x)
Example: [G,y] = planerot([3; 4]); y   % [5; 0]
Differences from MATLAB: Real double two-element column input.

## mkpp
Construct a piecewise polynomial structure.
Syntax: pp = mkpp(breaks,coefs); pp = mkpp(breaks,coefs,dim)
Example: pp = mkpp([0 1 2], [1 0; 2 1]); pp.order   % 2
Differences from MATLAB: Finite increasing real breaks and real coefficients; dim must be a positive scalar integer.

## unmkpp
Extract the fields of a piecewise polynomial structure.
Syntax: [breaks,coefs,pieces,order,dim] = unmkpp(pp)
Example: unmkpp(mkpp([0 1], [3 2 1]))   % [0 1]
Differences from MATLAB: Validates scalar pp structures with real coefficients and scalar output dimension.

## ppval
Evaluate a piecewise polynomial, extending the end pieces outside the breaks.
Syntax: Y = ppval(pp,X)
Example: ppval(mkpp([0 1 2], [1 0; 2 1]), [-1 3])   % [-1 5]
Differences from MATLAB: Accepts scalar pp structures with real coefficients. The existing two-argument spline and pchip functions do not yet create pp structures.

## residue
Convert rational polynomials to partial fractions or back.
Syntax: [r,p,k] = residue(b,a); [b,a] = residue(r,p,k)
Example: [r,p,k] = residue([1 2], [1 3 2]); p   % [-2; -1]
Differences from MATLAB: Coefficient and pole vectors are bounded at 4097 entries; close poles are grouped with relative tolerance 1e-3. With complex input, results with zero imaginary parts are real and [b,a] = residue(r,p,k) keeps leading zeros of b, as in MATLAB.

## blkdiag
Assemble matrices into a block diagonal matrix.
Syntax: B = blkdiag(A1,A2,...)
Example: blkdiag([1 2], 3)   % [1 2 0; 0 0 3]
Differences from MATLAB: Two-dimensional numeric, logical or character input; sparse input is not supported.

## cdf2rdf
Convert conjugate eigenvalue pairs to real block diagonal form.
Syntax: [Vr,Dr] = cdf2rdf(V,D)
Example: [Vr,Dr] = cdf2rdf(eye(2), diag([2 3])); Dr   % [2 0; 0 3]
Differences from MATLAB: Conjugate eigenvalues must be adjacent; the existing eig function does not yet return nonsymmetric eigenvectors.

## rsf2csf
Convert a real Schur form to a complex triangular Schur form.
Syntax: [Uc,Tc] = rsf2csf(U,T)
Example: [Uc,Tc] = rsf2csf(eye(2), [1 2; 0 3]); Tc   % [1 2; 0 3]
Differences from MATLAB: Dense square double or complex matrices; input must already represent a Schur form.

## besselj
Bessel function of the first kind.
Syntax: J = besselj(nu, Z)
Syntax: J = besselj(nu, Z, scale)
J_nu(Z) for real order nu and real or complex Z; scale 1 multiplies by exp(-abs(imag(Z))). A row of orders against a column of arguments gives a table. Negative real Z gives the principal-branch (complex) value.
Example: besselj(0, 1)   % 0.7652
Differences from MATLAB: one output (no ierr). Real arguments use Temme's series and Steed's continued fractions; complex arguments off the axes use power series below abs(Z) = 17, which lose accuracy as abs(Z) grows there. Orders above 1e4 return 0 (J, I) or Inf (Y, K) for abs(Z) <= nu/2 as MATLAB does, and NaN for larger abs(Z).

## bessely
Bessel function of the second kind.
Syntax: Y = bessely(nu, Z)
Syntax: Y = bessely(nu, Z, scale)
Y_nu(Z) for real order nu and real or complex Z; scale 1 multiplies by exp(-abs(imag(Z))). bessely(nu, 0) is -Inf.
Example: bessely(0, 1)   % 0.0883
Differences from MATLAB: one output (no ierr); complex arguments off the axes use power series below abs(Z) = 17.

## besseli
Modified Bessel function of the first kind.
Syntax: I = besseli(nu, Z)
Syntax: I = besseli(nu, Z, scale)
I_nu(Z) for real order nu and real or complex Z; scale 1 multiplies by exp(-abs(real(Z))).
Example: besseli(0, 1)   % 1.2661
Differences from MATLAB: one output (no ierr); complex arguments off the axes use power series below abs(Z) = 17.

## besselk
Modified Bessel function of the second kind.
Syntax: K = besselk(nu, Z)
Syntax: K = besselk(nu, Z, scale)
K_nu(Z) for real order nu and real or complex Z; scale 1 multiplies by exp(Z). besselk(nu, 0) is Inf.
Example: besselk(0, 1)   % 0.4210
Differences from MATLAB: one output (no ierr); complex arguments off the axes use power series below abs(Z) = 17.

## besselh
Bessel function of the third kind (Hankel function).
Syntax: H = besselh(nu, Z)
Syntax: H = besselh(nu, K, Z)
Syntax: H = besselh(nu, K, Z, scale)
H1 = J + iY for K = 1 (the default), H2 = J - iY for K = 2; scale 1 multiplies by exp(-i*Z) (K = 1) or exp(i*Z) (K = 2).
Example: besselh(0, 1)   % 0.7652 + 0.0883i
Differences from MATLAB: one output (no ierr); computed as J +- iY, so it loses accuracy where both grow and H decays (large imaginary Z).

## airy
Airy functions.
Syntax: W = airy(Z)
Syntax: W = airy(K, Z)
Syntax: W = airy(K, Z, scale)
K = 0 gives Ai (the default), 1 Ai', 2 Bi, 3 Bi'. scale 1 multiplies Ai and Ai' by exp(2/3*Z^(3/2)) and Bi and Bi' by exp(-abs(real(2/3*Z^(3/2)))).
Example: airy(0)   % 0.3550
Differences from MATLAB: one output (no ierr).

## gammainc
Regularized incomplete gamma function.
Syntax: Y = gammainc(X, A)
Syntax: Y = gammainc(X, A, tail)
P(A, X) for X >= 0 and A >= 0; tail is 'lower' (default), 'upper', 'scaledlower' or 'scaledupper' (the tail divided by X^A*exp(-X)/gamma(A+1)). Negative X gives a complex result, as in MATLAB.
Example: gammainc(1, 2)   % 0.2642
Differences from MATLAB: double inputs only; single is computed and returned as double.

## gammaincinv
Inverse of the regularized incomplete gamma function.
Syntax: X = gammaincinv(Y, A)
Syntax: X = gammaincinv(Y, A, tail)
Solves gammainc(X, A, tail) = Y for Y in [0, 1]; tail is 'lower' (default) or 'upper'.
Example: gammaincinv(0.5, 2)   % 1.6783
Differences from MATLAB: double inputs only.

## betainc
Regularized incomplete beta function.
Syntax: I = betainc(X, Z, W)
Syntax: I = betainc(X, Z, W, tail)
I_X(Z, W) for X in [0, 1] and Z, W >= 0; tail is 'lower' (default) or 'upper'.
Example: betainc(0.5, 2, 3)   % 0.6875
Differences from MATLAB: double inputs only.

## betaincinv
Inverse of the regularized incomplete beta function.
Syntax: X = betaincinv(Y, Z, W)
Syntax: X = betaincinv(Y, Z, W, tail)
Solves betainc(X, Z, W, tail) = Y for Y in [0, 1]; tail is 'lower' (default) or 'upper'.
Example: betaincinv(0.5, 2, 3)   % 0.3857
Differences from MATLAB: double inputs only.

## expint
Exponential integral E1.
Syntax: Y = expint(X)
E1(X) = integral from X to Inf of exp(-t)/t for real or complex X. Negative real X gives -Ei(-X) - i*pi. As in MATLAB, expint(Inf) is NaN and expint(NaN) is 0.
Example: expint(1)   % 0.2194
Differences from MATLAB: none known.

## psi
Digamma and polygamma functions.
Syntax: Y = psi(X)
Syntax: Y = psi(K, X)
psi(X) is the derivative of gammaln; psi(K, X) is its K-th derivative. X must be real and nonnegative; psi(0) is -Inf.
Example: psi(1)   % -0.5772
Differences from MATLAB: none known.

## legendre
Associated Legendre functions.
Syntax: P = legendre(n, X)
Syntax: P = legendre(n, X, normalization)
Returns P_n^m(X) for m = 0..n, one row per m and one column per element of X, with the Condon-Shortley phase. normalization is 'unnorm' (default), 'sch' (Schmidt semi-normalized) or 'norm' (fully normalized).
Example: legendre(2, 0.5)   % [-0.1250; -1.2990; 2.2500]
Differences from MATLAB: X must be a vector or scalar; a matrix X (a 3-D result in MATLAB) is rejected. Degrees above 10000 are rejected.

## ellipke
Complete elliptic integrals of the first and second kind.
Syntax: K = ellipke(M)
Syntax: [K, E] = ellipke(M, tol)
Uses the arithmetic-geometric mean (DLMF 19.8), which stops when every element is within tol (default eps). M must be in [0, 1]; ellipke(1) is Inf.
Example: ellipke(0.5)   % 1.8541
Differences from MATLAB: double inputs only.

## ellipj
Jacobi elliptic functions.
Syntax: [SN, CN, DN] = ellipj(U, M)
Syntax: [SN, CN, DN] = ellipj(U, M, tol)
Uses the descending Landen (AGM) method of DLMF 22.20 with tolerance tol (default eps). U and M must have the same size or one must be scalar; M must be in [0, 1].
Example: [sn, cn, dn] = ellipj(0.5, 0.3)   % sn = 0.4742
Differences from MATLAB: double inputs only.

## erfcx
Scaled complementary error function.
Syntax: Y = erfcx(X)
exp(X.^2) .* erfc(X), computed without overflow for large X. X must be real; single input gives single output.
Example: erfcx(1)   % 0.4276
Differences from MATLAB: none known.

## containers.Map
Create a map from keys to values (a handle object).
Syntax: m = containers.Map(keys, values)
Syntax: m = containers.Map('KeyType', kt, 'ValueType', vt)
Syntax: m = containers.Map(keys, values, 'UniformValues', tf)
Keys are char rows or real scalars (double, single, int32, uint32, int64, uint64), kept sorted. m(key) reads a value and m(key) = value adds or replaces one. Properties: Count (uint64), KeyType, ValueType. Copies such as m2 = m share one map. size(m) is [Count 1].
Example: m = containers.Map({'a','b'}, {1, 2}); m('b')   % 2
Example: m = containers.Map(); m('k') = 7; m2 = m; m2('j') = 8; m.Count   % 2
Differences from MATLAB: none known.

## keys
Keys of a containers.Map, in sorted order.
Syntax: k = keys(m)
Returns a 1-by-Count cell array. Char keys sort by character code, so uppercase comes before lowercase.
Example: m = containers.Map({'b','a'}, {1, 2}); keys(m)   % {'a', 'b'}
Differences from MATLAB: none known.

## values
Values of a containers.Map.
Syntax: v = values(m)
Syntax: v = values(m, keySet)
Without keySet, the values in key order as a 1-by-Count cell. keySet is a cell of keys; the result has its shape. A missing key is an error.
Example: m = containers.Map({'b','a'}, {1, 2}); values(m, {'a'})   % {2}
Differences from MATLAB: none known.

## isKey
True for keys present in a containers.Map.
Syntax: tf = isKey(m, key)
Syntax: tf = isKey(m, keySet)
A cell keySet gives a logical array of its shape. A key of another type is reported absent.
Example: m = containers.Map({'a'}, {1}); isKey(m, {'a','z'})   % [true false]
Differences from MATLAB: none known.

## remove
Delete keys from a containers.Map.
Syntax: m = remove(m, key)
Syntax: m = remove(m, keySet)
The map changes in place (it is a handle) and is also returned. A missing key gives a warning.
Example: m = containers.Map({'a','b'}, {1, 2}); remove(m, 'a'); m.Count   % 1
Differences from MATLAB: none known.

## inputParser
Create a parser for function inputs (a handle object).
Syntax: p = inputParser
Define arguments with addRequired, addOptional and addParameter, then call parse. Properties: FunctionName, CaseSensitive, KeepUnmatched, PartialMatching, StructExpand (settable), and Parameters, Results, Unmatched, UsingDefaults (read-only). copy(p) makes an independent parser.
Example: p = inputParser; p.addRequired('x'); p.parse(5); p.Results.x   % 5
Differences from MATLAB: Unmatched names must be valid MATLAB field names.

## addRequired
Add a required positional argument to an inputParser.
Syntax: addRequired(p, name)
Syntax: addRequired(p, name, validator)
Also called as p.addRequired(...). validator is a function handle; it passes when it returns a nonzero scalar or nothing, and fails when it returns anything else or raises an error.
Example: p = inputParser; addRequired(p, 'x', @isnumeric); parse(p, 2); p.UsingDefaults   % empty 1-by-0 cell
Differences from MATLAB: none known.

## addOptional
Add an optional positional argument to an inputParser.
Syntax: addOptional(p, name, default)
Syntax: addOptional(p, name, default, validator)
When parameters are also defined, a char argument goes to the optional argument only if its validator accepts it; otherwise it starts the name-value pairs.
Example: p = inputParser; addOptional(p, 'n', 3); parse(p); p.Results.n   % 3
Differences from MATLAB: none known.

## addParameter
Add a name-value parameter to an inputParser.
Syntax: addParameter(p, name, default)
Syntax: addParameter(p, name, default, validator, 'PartialMatchPriority', n)
Names match case-insensitively unless CaseSensitive is set, and by unique prefix while PartialMatching is set. Among several prefix matches the lowest PartialMatchPriority wins, with a warning.
Example: p = inputParser; addParameter(p, 'Tol', 1e-6); parse(p, 'Tol', 0.5); p.Results.Tol   % 0.5000
Differences from MATLAB: none known.

## addParamValue
Add a name-value parameter to an inputParser (older name of addParameter).
Syntax: addParamValue(p, name, default)
Syntax: addParamValue(p, name, default, validator)
Behaves like addParameter.
Example: p = inputParser; addParamValue(p, 'Mode', 'fast'); parse(p, 'mode', 'slow'); p.Results.Mode   % slow
Differences from MATLAB: MATLAB's which does not find addParamValue; MatRust lists it as a function.

## parse
Parse function inputs with an inputParser.
Syntax: parse(p, args...)
Fills p.Results (fields sorted by name), p.UsingDefaults and, with KeepUnmatched, p.Unmatched. Each parse starts again from the defaults. With StructExpand set, a scalar struct argument supplies name-value pairs. Errors name FunctionName when it is set.
Example: p = inputParser; p.addRequired('a'); p.addParameter('b', 0); p.parse(1); p.UsingDefaults   % {'b'}
Differences from MATLAB: none known.

## zip
Compress files into a ZIP archive.
Syntax: zip(zipfilename, files)
Syntax: entrynames = zip(zipfilename, files, rootfolder)
files is a name or a cell array of names; folders are added recursively and * or ? wildcards match in the last name part. Names are relative to rootfolder (default: the current folder) and stored with / separators. '.zip' is appended when the name has another extension. Entries use Deflate compression.
Example: d=tempname(pwd); mkdir(d); fid=fopen(fullfile(d,'a.txt'),'w'); fprintf(fid,'hi'); fclose(fid); names=zip(fullfile(d,'a.zip'),'a.txt',d); rmdir(d,'s'); names{1}   % a.txt
Differences from MATLAB: A file given by an absolute path outside the root folder is stored under its own name only. Folder entries are not stored; at most 10000 files and 256 MiB in total.

## unzip
Extract the files of a ZIP archive.
Syntax: unzip(zipfilename)
Syntax: filenames = unzip(zipfilename, outputfolder)
Extracts into outputfolder (created when missing) or the current folder and returns the extracted file names, last entry first, as MATLAB does. Entry names that would leave the output folder are refused.
Example: d=tempname(pwd); mkdir(d); fid=fopen(fullfile(d,'a.txt'),'w'); fprintf(fid,'hi'); fclose(fid); zip(fullfile(d,'a.zip'),'a.txt',d); unzip(fullfile(d,'a.zip'),fullfile(d,'out')); t=fileread(fullfile(d,'out','a.txt')); rmdir(d,'s'); t   % hi
Differences from MATLAB: URLs are not supported; at most 10000 entries and 256 MiB of extracted data.

## gzip
Compress files with GNU zip.
Syntax: gzip(files)
Syntax: gzfilenames = gzip(files, outputfolder)
Writes name.gz next to each file, or into outputfolder (created when missing). Folder inputs include their files recursively; an explicit outputfolder receives the compressed files without their source folder hierarchy. files may also use * or ? wildcards.
Example: d=tempname(pwd); mkdir(d); f=fullfile(d,'a.txt'); fid=fopen(f,'w'); fprintf(fid,'hi'); fclose(fid); g=gzip(f); ok=exist(g{1},'file'); rmdir(d,'s'); ok   % 2
Differences from MATLAB: Files are limited to 256 MiB each.

## gunzip
Uncompress GNU zip files.
Syntax: gunzip(files)
Syntax: filenames = gunzip(files, outputfolder)
Removes the last extension from each input name and writes next to the archive or into outputfolder: odd.bin becomes odd and t.tgz becomes t.
Example: d=tempname(pwd); mkdir(d); f=fullfile(d,'a.txt'); fid=fopen(f,'w'); fprintf(fid,'hi'); fclose(fid); g=gzip(f); delete(f); gunzip(g{1}); t=fileread(f); rmdir(d,'s'); t   % hi
Differences from MATLAB: Uncompressed output is limited to 256 MiB per file; URLs are not supported.

## tar
Pack files into a tar archive.
Syntax: tar(tarfilename, files)
Syntax: entrynames = tar(tarfilename, files, rootfolder)
Writes ustar headers with GNU long-name records when a UTF-8 entry name does not fit the header. A name ending in .tgz or .tar.gz is also GNU zip compressed, and '.tar' is appended to other names. files works as in zip.
Example: d=tempname(pwd); mkdir(d); fid=fopen(fullfile(d,'a.txt'),'w'); fprintf(fid,'hi'); fclose(fid); names=tar(fullfile(d,'a.tgz'),'a.txt',d); rmdir(d,'s'); names{1}   % a.txt
Differences from MATLAB: At most 10000 files and 256 MiB in total.

## untar
Extract the files of a tar archive.
Syntax: untar(tarfilename)
Syntax: filenames = untar(tarfilename, outputfolder)
Reads plain or GNU zip compressed ustar archives into outputfolder or the current folder and returns the file names in archive order. Entry names that would leave the output folder are refused.
Example: d=tempname(pwd); mkdir(d); fid=fopen(fullfile(d,'a.txt'),'w'); fprintf(fid,'hi'); fclose(fid); tar(fullfile(d,'a.tar'),'a.txt',d); n=untar(fullfile(d,'a.tar'),fullfile(d,'out')); t=fileread(fullfile(d,'out','a.txt')); rmdir(d,'s'); t   % hi
Differences from MATLAB: Links and other special entries are skipped; URLs are not supported.

## importdata
Load data from a text, image, audio or MAT file.
Syntax: A = importdata(filename)
Syntax: [A, delimiter, headerlines] = importdata(filename, delimiter, headerlines)
Numeric text gives a matrix (short rows padded with NaN). Header lines give a struct with data, textdata and colheaders; leading row labels give data, textdata and rowheaders; text only gives a cell column of lines. The delimiter is detected (comma, tab, then spaces) unless given. PNG files return the image, WAV files a struct with data and fs, and MAT files the variable (or a struct of all variables).
Example: d=tempname(pwd); mkdir(d); f=fullfile(d,'n.csv'); fid=fopen(f,'w'); fprintf(fid,'1,2\n3,4\n'); fclose(fid); A=importdata(f); rmdir(d,'s'); A   % [1 2; 3 4]
Differences from MATLAB: Only the image and audio formats of imread and audioread; spreadsheets and the clipboard are not supported yet.

## imread
Read a PNG image.
Syntax: A = imread(filename)
Syntax: [X, map, alpha] = imread(filename, fmt)
Grayscale images return uint8 or uint16 (1-bit images logical, 2- and 4-bit images scaled to uint8); indexed images return zero-based uint8 indices and a colormap; the third output is the alpha channel or [].
Example: f=[tempname(pwd) '.png']; imwrite(uint8([1 2; 3 4]), f); A=imread(f); delete(f); A   % [1 2; 3 4] uint8
Differences from MATLAB: Only PNG; JPEG, TIFF, BMP and GIF are not supported yet. Truecolor (RGB) images need N-D arrays, which are not supported yet; interlaced PNG is not supported yet.

## imwrite
Write a PNG image.
Syntax: imwrite(A, filename)
Syntax: imwrite(X, map, filename, fmt, 'BitDepth', d, 'Alpha', alpha)
Writes grayscale (double, single, logical, uint8, uint16) or indexed images. Doubles map 0..1 to the full range (NaN and values above 1 give the maximum, as MATLAB does). BitDepth 1, 2, 4, 8 or 16; other text parameters become PNG text chunks.
Example: f=[tempname(pwd) '.png']; imwrite([0 0.5 1], f); A=imread(f); delete(f); A   % [0 128 255] uint8
Differences from MATLAB: Only PNG; JPEG, TIFF, BMP and GIF are not supported yet, nor truecolor (N-D) data or the Transparency, Background and Gamma parameters.

## imfinfo
Information about a PNG file.
Syntax: info = imfinfo(filename)
Syntax: info = imfinfo(filename, fmt)
Returns MATLAB's PNG field set: Filename, FileModDate, FileSize, Format, Width, Height, BitDepth, ColorType, Colormap, InterlaceType, Transparency, text fields and others ([] when absent).
Example: f=[tempname(pwd) '.png']; imwrite(uint8(magic(4)), f); i=imfinfo(f); delete(f); i.ColorType   % grayscale
Differences from MATLAB: Only PNG files.

## audioread
Read a WAV audio file.
Syntax: [y, Fs] = audioread(filename)
Syntax: [y, Fs] = audioread(filename, [start finish], datatype)
Returns samples by channels. 'double' (default) scales to [-1, 1); 'native' keeps uint8, int16, int32 (24-bit data shifted left by 8) or single.
Example: f=[tempname(pwd) '.wav']; audiowrite(f, [0; 0.5], 8000); y=audioread(f); delete(f); y   % [0; 0.5]
Differences from MATLAB: Only WAV files (PCM 8/16/24/32-bit and 32/64-bit float); FLAC, Ogg and MPEG-4 are not supported yet.

## audiowrite
Write a WAV audio file.
Syntax: audiowrite(filename, y, Fs)
Syntax: audiowrite(filename, y, Fs, 'BitsPerSample', b, 'Title', t, 'Artist', a, 'Comment', c)
y is samples by channels (a row vector is one channel) of class uint8, int16, int32, single or double. BitsPerSample is 8, 16 (default), 24, 32 or 64; 32 and 64 write floating point (int32 data writes 32-bit integers). Floating-point data outside [-1, 1] is clipped with a warning.
Example: f=[tempname(pwd) '.wav']; audiowrite(f, [0; 1], 8000); y=audioread(f, 'native'); delete(f); y   % [0; 32767] int16
Differences from MATLAB: Only .wav files; .flac, .ogg, .m4a and .mp4 are not supported yet.

## audioinfo
Information about a WAV audio file.
Syntax: info = audioinfo(filename)
Returns Filename, CompressionMethod, NumChannels, SampleRate, TotalSamples, Duration, Title, Comment, Artist ([] when absent) and BitsPerSample.
Example: f=[tempname(pwd) '.wav']; audiowrite(f, zeros(8,2), 8000); i=audioinfo(f); delete(f); i.NumChannels   % 2
Differences from MATLAB: Only WAV files.

## fileattrib
Get or set file and folder attributes.
Syntax: [status, values, msgid] = fileattrib(name)
Syntax: [status, msg, msgid] = fileattrib(name, attribs, users)
values has Name, archive, system, hidden, directory and the User, Group and Other read/write/execute fields; name may end in a wildcard (a struct array). With no outputs and no attribs, the attributes are displayed; bare fileattrib queries the current folder. attribs such as '+w' or '-h' set or clear attributes: a, h, s, w on Windows, r, w, x (users u, g, o, a) elsewhere. With outputs, failures return status 0 and a message instead of an error.
Example: s=fileattrib(pwd)   % 1
Differences from MATLAB: The recursive 's' option is not supported yet.

## bsxfun
Apply a two-input function with singleton expansion.
Syntax: C = bsxfun(fun, A, B)
fun must be a function handle. A dimension of size 1 in one input is repeated to match the other input. Handles to built-in functions such as @plus receive the expanded arrays at once; user functions and anonymous handles receive one column (or a column and a scalar) at a time.
Example: bsxfun(@plus, [1 2 3], [10; 20])   % [11 12 13; 21 22 23]
Differences from MATLAB: Two-dimensional arrays only.

## builtin
Call the built-in version of a function.
Syntax: [varargout] = builtin(name, args...)
name must name a function MATLAB implements as a built-in; toolbox M-files such as mean are rejected. A user function of the same name is skipped, so an overload can call the built-in.
Example: builtin('sum', [1 2 3])   % 6
Differences from MATLAB: none known for these forms.

## which
Locate a function or variable.
Syntax: s = which(name); c = which(name, '-all'); which name
Returns 'variable' for a workspace variable, the full path of a function file in the current folder, built-in (...) for built-in functions and keywords, and '' when nothing is found.
Example: s = which('zz_nonexistent_fn')   % ''
Differences from MATLAB: Built-in and toolbox locations are reported under the MatRust installation folder; '-all' lists one location.

## what
List the MATLAB files in a folder.
Syntax: s = what; s = what(folder); what folder
Returns a struct with fields path, m, mlapp, mat, mex, mdl, slx, p, classes and packages; each list is a sorted column cell. A missing folder gives a 0x1 struct.
Example: w = what(pwd); class(w.m)   % cell
Differences from MATLAB: Folders are looked up from the current folder only, not on a search path.

## isobject
True for MATLAB objects.
Syntax: tf = isobject(A)
True for MException, onCleanup and inline objects; false for numbers, structs, cells and function handles.
Example: isobject(MException('a:b', 'c'))   % true
Differences from MATLAB: none known for these classes.

## ismethod
True if a name is a method of an object's class.
Syntax: tf = ismethod(obj, name)
obj may also be a class name such as 'containers.Map'. Methods of numeric, logical and char values are the supported built-in functions.
Example: ismethod(inline('t'), 'formula')   % true
Differences from MATLAB: Method lists are fixed tables for inline, MException, containers.Map and the built-in value classes.

## iskeyword
True for MATLAB keywords.
Syntax: tf = iskeyword(s); list = iskeyword
Without an input it returns the 20 keywords as a column cell.
Example: iskeyword('for')   % true
Differences from MATLAB: none known.

## genvarname
Construct a valid, unique variable name.
Syntax: name = genvarname(candidate); name = genvarname(candidate, protected)
Spaces are removed and the next letter capitalized, other invalid characters become 0xHH codes, an initial non-letter or a keyword gets an x prefix, and a number is appended until the name differs from protected names. A cell input gives a cell of mutually unique names.
Example: genvarname('my var')   % myVar
Differences from MATLAB: none known for these forms.

## namelengthmax
Maximum length of a MATLAB name.
Syntax: n = namelengthmax
Example: namelengthmax   % 63
Differences from MATLAB: none known.

## clearvars
Clear selected workspace variables.
Syntax: clearvars v1 v2 ...; clearvars ... -except k1 k2 ...; clearvars -regexp pattern ...; clearvars -global ...
Names may contain * wildcards. -regexp makes the following names regular expressions; -except keeps matching variables; -global selects among all global variables, including those declared only inside functions, and clears their global values.
Example: a = 1; b = 2; clearvars a; exist('a')   % 0
Differences from MATLAB: none known for these forms.

## functions
Information about a function handle.
Syntax: s = functions(fh)
Named handles give fields function, type ('simple') and file. A handle made inside a file to one of its local functions has type 'scopedfunction', that file and parentage. Anonymous handles add workspace (a cell holding a struct of the captured variables) and within_file_path.
Example: s = functions(@(t) t + 1); s.type   % anonymous
Differences from MATLAB: Nested functions are not supported, so there are no 'nested' handles.

## localfunctions
Handles to the local functions of the running file.
Syntax: c = localfunctions
Inside a function file it returns a column cell of handles to the file's local functions in file order; elsewhere it returns {}.
Example: localfunctions   % {}
Differences from MATLAB: A handle made by localfunctions is resolved by name when it is called.

## dbstack
Function call stack.
Syntax: st = dbstack; st = dbstack(n); st = dbstack('-completenames'); [st, k] = dbstack; dbstack
Returns a column struct array with fields file, name and line, innermost function first; n omits that many frames. At the command line the stack is empty.
Example: numel(dbstack)   % 0
Differences from MATLAB: Scripts and anonymous functions do not appear as frames.

## isequalwithequalnans
Equality that treats NaN values as equal.
Syntax: tf = isequalwithequalnans(A, B, ...)
The same as isequaln.
Example: isequalwithequalnans([1 NaN], [1 NaN])   % true
Differences from MATLAB: none known.

## bitmax
Removed function; use flintmax.
Syntax: bitmax
Every call raises MATLAB's error "BITMAX has been removed. Use FLINTMAX instead."
Example: flintmax   % 9.0072e+15
Differences from MATLAB: none known.

## inline
Construct an inline function object.
Syntax: f = inline(expr); f = inline(expr, arg1, arg2, ...); f = inline(expr, n)
Without names the arguments are the variables found in expr, in alphabetical order (x when there are none); n gives x, P1, ..., Pn. Call it as f(args) or feval(f, args); formula, argnames, char, vectorize and nargin inspect it.
Example: f = inline('t^2 + 1'); f(3)   % 10
Differences from MATLAB: Display inside cells and structs shows the underlying fields.

## argnames
Argument names of an inline function.
Syntax: c = argnames(f)
Example: argnames(inline('a*x + b'))   % {'a'; 'b'; 'x'}
Differences from MATLAB: none known.

## formula
Expression of an inline function.
Syntax: s = formula(f)
Example: formula(inline('sin(t)'))   % sin(t)
Differences from MATLAB: none known.

## mexext
MEX-file extension of the platform.
Syntax: ext = mexext; s = mexext('all')
mexext('all') returns a 1x4 struct with fields arch and ext.
Example: s = mexext('all'); s(4).ext   % mexw64
Differences from MATLAB: MatRust cannot load MEX files.

## maxNumCompThreads
Number of computational threads (deprecated).
Syntax: n = maxNumCompThreads; old = maxNumCompThreads(n); old = maxNumCompThreads('automatic')
Every call warns that the function will be removed. The query returns the stored setting, by default the number of processors.
Example: n = maxNumCompThreads;   % number of threads, with a deprecation warning
Differences from MATLAB: The setting is only stored; it does not change how many workers MatRust uses.

## usejava
True if a Java feature is available.
Syntax: tf = usejava(feature)
Example: usejava('jvm')   % false
Differences from MATLAB: MatRust has no Java, so every feature is false.

## isstudent
True for the MATLAB Student Version.
Syntax: tf = isstudent
Example: isstudent   % false
Differences from MATLAB: none known.

## diary
Record command-window output in a file.
Syntax: diary filename; diary(filename); diary on; diary off; diary
Output is appended to the file while the diary is on; diary without arguments toggles it. get(0, 'Diary') and get(0, 'DiaryFile') report the state. A file that cannot be opened or written is an error and leaves the diary off.
Example: diary off; get(0, 'Diary')   % off
Differences from MATLAB: The commands typed are not recorded, only the output. Output captured by evalc is recorded too (MATLAB turns the diary off inside evalc).

## ode113
Solve nonstiff differential equations, variable order (Adams-Bashforth-Moulton PECE).
Syntax: [t, y] = ode113(@(t,y) -y, tspan, y0)
Syntax: [t, y] = ode113(@f, tspan, y0, options, p1, p2, ...)
Syntax: [t, y, te, ye, ie] = ode113(@f, tspan, y0, odeset('Events', @ev))
Syntax: sol = ode113(@f, tspan, y0)
Integrates dy/dt = f(t, y) with a variable-step, variable-order (1 to 12) Adams
predictor-corrector using modified divided differences and local extrapolation,
as described by Shampine and Gordon (1975) and Hairer, Norsett and Wanner (1993,
sections III.5 and III.7). Error estimates select the step size and order; steps,
statistics and numerical values can differ from MATLAB.
tspan = [t0 tf] returns the solver steps; a longer tspan returns y at exactly those
times, interpolated. Options from odeset: RelTol, AbsTol, NormControl, MaxStep,
InitialStep, Refine, Stats, Events, Mass. Extra arguments after options are passed to f.
One output with a handle gives a solution structure for deval and odextend.
Example: [t, y] = ode113(@(t,y) -y, [0 1], 1); y(end)   % 0.3679
Differences from MATLAB: the options OutputFcn, OutputSel and NonNegative stop with an
error. sol.idata keeps phi3d as a 2-D neq-by-(14*N) matrix.

## ode15s
Solve stiff differential equations and DAEs, variable order (NDF/BDF 1 to 5).
Syntax: [t, y] = ode15s(@f, tspan, y0)
Syntax: [t, y] = ode15s(@f, tspan, y0, odeset('Jacobian', J, 'Mass', M))
Syntax: [t, y, te, ye, ie] = ode15s(@f, tspan, y0, odeset('Events', @ev))
Syntax: sol = ode15s(@f, tspan, y0)
Uses the numerical differentiation formulas of Shampine and Reichelt (1997,
section 2), or backward differentiation formulas with odeset('BDF','on'),
of orders 1 through MaxOrder (at most 5). Backward differences provide error
estimates and polynomial interpolation; quasi-constant steps and order changes
control the error. A simplified Newton iteration reuses a finite-difference or
user Jacobian and its linear-system factors when possible. Mass may be a constant
matrix or a function M(t, y). Constant diagonal singular mass matrices support
index-1 DAEs, with consistent initial values obtained by solving the algebraic
constraints (Shampine, Reichelt and Kierzenka, 1999, sections 3–4).
Stats 'on' prints the six counters.
Example: [t, y] = ode15s(@(t,y) [-y(1); -1000*y(2)], [0 1], [1; 1]); y(end, 1)   % approximately 0.368
Differences from MATLAB: adaptive steps, order choices, statistics and rounded
solution values can differ. Consistent initial conditions are computed only for
constant diagonal mass matrices. OutputFcn, OutputSel and NonNegative are not supported.

## ode23s
Solve stiff differential equations, low order (modified Rosenbrock 2(3)).
Syntax: [t, y] = ode23s(@f, tspan, y0)
Syntax: [t, y] = ode23s(@f, tspan, y0, odeset('Jacobian', @J, 'Mass', M))
Syntax: sol = ode23s(@f, tspan, y0)
Modified Rosenbrock (2,3) pair of Shampine and Reichelt (1997), used as a W-method: each
step forms the Jacobian (numerical unless the Jacobian option is given), a finite-difference
df/dt and one LU factorization of M - h*d*J, d = 1/(2+sqrt(2)); the third stage gives the
error estimate and the solution has a quadratic continuous extension. The mass matrix must be
constant and nonsingular.
Example: [t, y] = ode23s(@(t,y) -y, [0 1], 1); y(end)   % about 0.3677
Differences from MATLAB: the initial step and step-size controller are this implementation's
own, so steps, statistics and the last digits of solution values can differ.

## ode23t
Solve moderately stiff differential equations and DAEs (trapezoidal rule).
Syntax: [t, y] = ode23t(@f, tspan, y0)
Syntax: [t, y] = ode23t(@f, tspan, y0, odeset('Mass', M))
Syntax: sol = ode23t(@f, tspan, y0)
The trapezoidal rule, solved for the scaled derivative h*y' by a simplified Newton iteration
whose Jacobian is refreshed only when the iteration fails to converge. The local error is
estimated from the difference between the corrector and an explicit second-order predictor
that uses the previous step's derivative (Milne's device); the solution between steps is the
cubic Hermite interpolant. Constant, singular diagonal or function mass matrices are accepted.
Example: [t, y] = ode23t(@(t,y) -y, [0 1], 1); y(end)   % about 0.368
Differences from MATLAB: the error estimate, initial step and iteration-matrix updates are this
implementation's own, so steps, statistics and the last digits of solution values can differ.

## ode23tb
Solve stiff differential equations (TR-BDF2, implicit Runge-Kutta).
Syntax: [t, y] = ode23tb(@f, tspan, y0)
Syntax: [t, y] = ode23tb(@f, tspan, y0, odeset('Mass', @(t,y) M))
Syntax: sol = ode23tb(@f, tspan, y0)
TR-BDF2 (Bank et al. 1985; Hosea and Shampine 1996): a trapezoidal stage to t + (2-sqrt(2))h
followed by a BDF2 stage, both solved with the same iteration matrix, and the error estimate of
the companion third-order formula. The solution between steps is the quadratic through the
step's three solution values. Constant or function mass matrices are accepted.
Example: [t, y] = ode23tb(@(t,y) -y, [0 1], 1); y(end)   % about 0.3677
Differences from MATLAB: the initial step and step-size controller are this implementation's
own, so steps, statistics and the last digits of solution values can differ.

## ode15i
Solve fully implicit differential equations f(t, y, y') = 0 (variable-order BDF).
Syntax: [t, y] = ode15i(@(t,y,yp) f, tspan, y0, yp0)
Syntax: [t, y] = ode15i(@f, tspan, y0, yp0, options)
Syntax: sol = ode15i(@f, tspan, y0, yp0)
Variable-order (1 to MaxOrder, at most 5) variable-coefficient backward differentiation
formulas for index-1 implicit equations, following Brenan, Campbell and Petzold (1996,
chapters 3 and 5) and the interpolation and initialisation mathematics of Shampine (2002).
Newton iterations use finite-difference Jacobians df/dy and df/dy'. Divided differences
control the step size and order. Use decic for consistent y0 and yp0. The interpolant
is the polynomial through the last k+1 solution points.
Example: [t, y] = ode15i(@(t,y,yp) yp + y, [0 1], 1, -1); y(end)   % approximately exp(-1)
Differences from MATLAB: values, steps, orders and statistics can differ. Events,
Jacobian, NormControl 'on' and OutputFcn options are not supported.

## decic
Compute consistent initial conditions for ode15i.
Syntax: [y0new, yp0new] = decic(@f, t0, y0, fixed_y0, yp0, fixed_yp0)
Syntax: [y0new, yp0new, resnrm] = decic(@f, t0, y0, fixed_y0, yp0, fixed_yp0, options)
Adjusts the components of y0 and yp0 whose fixed flags are 0 so that f(t0, y0, yp0) = 0,
by damped Newton iterations with finite-difference Jacobians. Singular value decomposition
gives minimum-norm corrections in the free components (Golub and Van Loan, Matrix
Computations, 2013, section 5.5). Fixed components remain unchanged; failure to reduce
the residual reports a convergence error. At most numel(y0) components may be fixed.
Example: [y0, yp0] = decic(@(t,y,yp) yp + y, 0, 1, 1, 0, 0)   % y0 = 1, yp0 = -1
Differences from MATLAB: nonlinear iteration paths, the chosen consistent solution and
the final residual can differ.

## deval
Evaluate the solution of a differential equation.
Syntax: sxint = deval(sol, xint)
Syntax: sxint = deval(sol, xint, idx)
Syntax: [sxint, spxint] = deval(sol, xint)
Syntax: sxint = deval(xint, sol)
Evaluates a solution structure from ode45, ode23, ode113, ode15s, ode23s, ode23t,
ode23tb, ode15i, bvp4c, bvp5c, dde23 or ddesd at the points xint with the solver's own
interpolant. Each column of sxint is the solution at one point; idx selects components.
The second output is the derivative.
Example: sol = ode45(@(t,y) -y, [0 1], 1); deval(sol, 0.5)   % 0.6065
Differences from MATLAB: error messages leave the argument name blank, as MATLAB does
inside evalc. The derivative of the stiff and ode113 interpolants is a central difference.

## odextend
Extend the solution of an initial value problem.
Syntax: solext = odextend(sol, odefun, tfinal)
Syntax: solext = odextend(sol, [], tfinal, yinit, options)
Continues the integration of sol from sol.x(end) to tfinal with the same solver (a new
odefun, initial values or options may be given) and returns the joined solution.
Example: sol = ode45(@(t,y) -y, [0 1], 1); sol2 = odextend(sol, [], 2); deval(sol2, 2)   % 0.1353
Differences from MATLAB: none known.

## bvpinit
Form the initial guess for bvp4c and bvp5c.
Syntax: solinit = bvpinit(x, yinit)
Syntax: solinit = bvpinit(x, @guess, parameters)
x is the initial mesh (monotonic, repeated points allowed); yinit is a constant vector or
a function returning the guess at a mesh point. Returns a structure with fields solver,
x, y, parameters (when given) and yinit.
Example: s = bvpinit([0 0.5 1], [1 2]); s.y   % [1 1 1; 2 2 2]
Differences from MATLAB: extending a previous solution (bvpinit(sol, ...)) is not
supported yet; use bvpxtend.

## bvpset
Create or alter options for bvp4c and bvp5c.
Syntax: options = bvpset('name1', value1, ...)
Syntax: options = bvpset(oldopts, 'name1', value1, ...)
Syntax: options = bvpset(oldopts, newopts)
Fields: AbsTol, RelTol, SingularTerm, FJacobian, BCJacobian, Stats, Nmax, Vectorized.
Names match by case-insensitive unique prefix; unset fields are [].
Example: o = bvpset('RelTol', 1e-4); o.RelTol   % 1.0000e-04
Differences from MATLAB: bvpset with no arguments does not print the property list.

## bvpget
Get an option from a bvpset structure.
Syntax: v = bvpget(options, 'name')
Syntax: v = bvpget(options, 'name', default)
Returns the field, or the default (or []) when it is unset.
Example: bvpget(bvpset('RelTol', 1e-4), 'AbsTol', 1e-7)   % 1.0000e-07
Differences from MATLAB: none known.

## bvp4c
Solve boundary value problems (3-stage Lobatto IIIa collocation).
Syntax: sol = bvp4c(@odefun, @bcfun, solinit)
Syntax: sol = bvp4c(@odefun, @bcfun, solinit, options, p1, p2, ...)
The three-stage Lobatto IIIA method of Kierzenka and Shampine (2001) uses a C1
cubic collocated at the endpoints and midpoint of each interval. Damped Newton
iterations solve the collocation and boundary equations together. Five-point
Lobatto quadrature estimates each subinterval's L2 norm of the ODE residual,
scaled by max(abs(odefun), AbsTol/RelTol), for mesh refinement. Unknown parameters come from
solinit.parameters (odefun(x,y,p), bcfun(ya,yb,p)). sol has fields solver, x, y, yp,
parameters (if any) and stats; deval evaluates the cubic and its derivative.
Example: sol = bvp4c(@(x,y) [y(2); -y(1)], @(a,b) [a(1); b(1)-1], bvpinit(linspace(0,pi/2,5), [0 0])); deval(sol, 1)   % [0.8414; 0.5403]
Differences from MATLAB: mesh selection, stopping tests, residual estimates, values
and evaluation counts can differ. The dense pivoted LU solve allows at most 3000
unknowns. Multipoint problems and SingularTerm are not supported.

## bvp5c
Solve boundary value problems (4-stage Lobatto IIIa collocation).
Syntax: sol = bvp5c(@odefun, @bcfun, solinit)
Syntax: sol = bvp5c(@odefun, @bcfun, solinit, options)
The four-stage Lobatto IIIA method of Kierzenka and Shampine (2008, sections 2–3)
collocates a quartic at the four Lobatto points of each interval. Damped Newton
iterations solve for mesh values, interior stages and any unknown parameters.
The mesh is refined using the sampled ODE residual multiplied by interval length
and scaled relative to local solution magnitudes with RelTol and AbsTol. This
controls the error asymptotically for smooth, well-conditioned problems. sol has
fields solver, x, y, idata, parameters (if any) and stats (maxerr, the scaled
residual estimate); deval evaluates the quartic and its derivative.
Example: sol = bvp5c(@(x,y) [y(2); -y(1)], @(a,b) [a(1); b(1)-1], bvpinit(linspace(0,pi/2,5), [0 0])); deval(sol, 1)   % [0.8415; 0.5403]
Differences from MATLAB: mesh selection, stopping tests, error estimates, values
and evaluation counts can differ. The dense solve has the same 3000-unknown
limit as bvp4c, including interior stages. Multipoint problems and SingularTerm
are not supported.

## bvpxtend
Extend a boundary value solution to a longer interval.
Syntax: solinit = bvpxtend(sol, xnew)
Syntax: solinit = bvpxtend(sol, xnew, ynew)
Syntax: solinit = bvpxtend(sol, xnew, 'constant' | 'linear' | 'solution')
Adds the point xnew before or after the mesh with ynew, or with the end value
('constant', the default), a linear extrapolation using the end slope, or the last
interval's cubic extrapolated ('solution').
Example: sol = bvp4c(@(x,y) [y(2); -y(1)], @(a,b) [a(1); b(1)-1], bvpinit(linspace(0,pi/2,5), [0 0])); s = bvpxtend(sol, 2); s.x(end)   % 2
Differences from MATLAB: none known.

## dde23
Solve delay differential equations with constant delays.
Syntax: sol = dde23(@(t,y,Z) f, lags, history, tspan)
Syntax: sol = dde23(@f, lags, @history, tspan, options)
Uses the Bogacki-Shampine 2(3) pair with cubic Hermite history, following Shampine
and Thompson, "Solving DDEs in MATLAB" (2001), sections 2 and 4-7. Potential
discontinuities from initial history jumps and Jumps are propagated through three
delay additions and included in the mesh. A continuous restart from a previous
solution does not introduce a new initial jump. Steps longer than a delay use bounded fixed-point
iteration; unsuccessful steps are reduced. history is a constant vector, a
function of t, or a previous solution. Z(:,j) is y(t - lags(j)). sol has fields
solver, history, discont, x, y, stats and yp (and xe, ye, ie with Events).
Example: sol = dde23(@(t,y,Z) -Z, 1, 1, [0 2]); deval(sol, 2)   % -0.5000
Differences from MATLAB: step sizes, statistics and last-digit rounding can differ;
NormControl and OutputFcn are not supported.

## ddesd
Solve delay differential equations with general delays.
Syntax: sol = ddesd(@(t,y,Z) f, @(t,y) d, history, tspan)
Syntax: sol = ddesd(@f, lags, history, tspan, options)
The delays are a function returning the delayed times d(t, y) <= t, or constant lags.
Uses classical four-stage Runge-Kutta steps and a cubic Hermite interpolant with
sampled residual control, following the approach of Shampine, "Solving ODEs and
DDEs with residual control" (2005), sections 2.1 and 3. The residual compares the
interpolant's derivative with the differential equation at interior points.
Short delays use bounded fixed-point iteration. sol has fields solver, history,
x, y, stats and yp
(and xe, ye, ie with Events).
Example: sol = ddesd(@(t,y,Z) -Z, @(t,y) t - 1, 1, [0 2]); deval(sol, 1.5)   % -0.3750
Differences from MATLAB: residual sampling, step sizes, statistics and numerical
results can differ.

## ddeset
Create or alter options for dde23 and ddesd.
Syntax: options = ddeset('name1', value1, ...)
Syntax: options = ddeset(oldopts, 'name1', value1, ...)
Fields: AbsTol, Events, InitialStep, InitialY, Jumps, MaxStep, NormControl, OutputFcn,
OutputSel, Refine, RelTol, Stats. Names match by case-insensitive unique prefix.
Example: o = ddeset('RelTol', 1e-4); o.RelTol   % 1.0000e-04
Differences from MATLAB: ddeset with no arguments does not print the property list.

## ddeget
Get an option from a ddeset structure.
Syntax: v = ddeget(options, 'name')
Syntax: v = ddeget(options, 'name', default)
Returns the field, or the default (or []) when it is unset.
Example: ddeget(ddeset('RelTol', 1e-4), 'RelTol')   % 1.0000e-04

## fftn
N-dimensional discrete Fourier transform of a vector or matrix.
Syntax: Y = fftn(X)
Syntax: Y = fftn(X, [m n])
Transforms along both dimensions. A size vector pads with zeros or truncates first. Real input gives the Hermitian result of a real-to-complex transform; a result whose imaginary parts are all zero is real. Single input gives a single result.
Example: fftn([1 2; 3 4])   % [10 -2; -4 0]
Differences from MATLAB: MatRust has no N-D arrays: a size vector may not have entries other than 1 beyond the second. Integer input is rejected as in MATLAB. Lengths up to 4 reproduce MATLAB's rounding; longer dimensions use MatRust's fft, whose last-digit rounding is not FFTW's.

## ifftn
N-dimensional inverse discrete Fourier transform of a vector or matrix.
Syntax: X = ifftn(Y)
Syntax: X = ifftn(Y, [m n])
Syntax: X = ifftn(..., 'symmetric')
Transforms back along both dimensions and divides by the number of elements. Conjugate-symmetric input, or the 'symmetric' flag, gives a real result.
Example: ifftn([10 -2; -4 0])   % [1 2; 3 4]
Differences from MATLAB: Two-dimensional arrays only (no N-D arrays). 'symmetric' reads the first half of the spectrum (along dimension 1, or along the length of a vector) and rebuilds the rest by conjugate symmetry, as the MATLAB documentation describes.

## convn
N-dimensional convolution of vectors and matrices.
Syntax: C = convn(A, B)
Syntax: C = convn(A, B, shape)
shape is 'full' (default), 'same' or 'valid', as for conv2. An empty B gives zeros the size of A.
Example: convn([1 2 3], [1 1])   % [1 3 5 3]
Differences from MATLAB: Two-dimensional arrays only (no N-D arrays).

## interp3
Three-dimensional gridded interpolation.
Syntax: Vq = interp3(X, Y, Z, V, Xq, Yq, Zq)
MatRust has no N-D arrays, so every form is rejected. interp3(V, Xq, Yq, Zq) with a matrix V gives MATLAB's error for that call.
Example: try, interp3(magic(3), 1, 1, 1), catch e, disp(e.message), end   % Output argument "varargout{3}" (and maybe others) not assigned during call to "meshgridvectors".
Differences from MATLAB: Three-dimensional data is not supported (no N-D arrays).

## interpn
Interpolation on a one- or two-dimensional grid in ndgrid layout.
Syntax: Vq = interpn(X, V, Xq)
Syntax: Vq = interpn(X1, X2, V, Y1, Y2)
Syntax: Vq = interpn(V, Y1, Y2)
Syntax: Vq = interpn(V, ntimes)
Syntax: Vq = interpn(..., method, extrapval)
method is 'linear' (default), 'nearest', 'cubic', 'spline' or 'pchip'. Points outside the grid get extrapval (NaN by default). interpn(V, ntimes) refines the grid ntimes times between the points (once when ntimes is omitted). Query arrays of different shapes that are vectors are expanded as grid vectors.
Example: interpn([1 2 3], [10 20 30], 2.5)   % 25
Example: interpn(magic(3), 1.5, 2.5)   % 4.7500
Differences from MATLAB: One- and two-dimensional grids only (no N-D arrays). 'cubic' on a non-uniform grid uses 'spline' (not checked against MATLAB).

## griddata
Interpolate scattered two-dimensional data.
Syntax: vq = griddata(x, y, v, xq, yq)
Syntax: vq = griddata(x, y, v, xq, yq, method)
Syntax: [xq, yq, vq] = griddata(...)
method is 'linear' (default), 'nearest', 'natural', 'cubic' or 'v4'. A row xq with a column yq is expanded to a grid. Duplicate points are merged and their values averaged, with a warning. Query points outside the convex hull give NaN, except with 'v4'. 'cubic' is C1 (smooth across triangle edges) and reproduces linear data exactly.
Example: griddata([0 1 1 0 0.5], [0 0 1 1 0.5], [0 1 2 1 0.75], 0.25, 0.75)   % 0.8750
Example: griddata([0 1 1 0 0.5], [0 0 1 1 0.5], [1 3 6 4 3.5], 0.25, 0.75, 'cubic')   % 3.7500
Differences from MATLAB: 'cubic' is a Clough-Tocher interpolant with minimum norm network gradient estimates (Nielson, Renka and Cline). MATLAB's own gradient estimate is not identified, so results can differ from MATLAB's in the third decimal place. The three-dimensional form griddata(x, y, z, v, xq, yq, zq) is not supported.

## griddatan
Interpolate scattered data given as rows of points.
Syntax: yi = griddatan(X, y, XI)
Syntax: yi = griddatan(X, y, XI, method)
method is 'linear' (default) or 'nearest'. Linear interpolation outside the convex hull gives NaN.
Example: griddatan([0 0; 1 0; 0 1], [1; 2; 3], [0.2 0.2])   % 1.6000
Differences from MATLAB: Two-column (two-dimensional) points only.

## delaunay
Delaunay triangulation of two-dimensional points.
Syntax: T = delaunay(x, y)
Syntax: T = delaunay(P)
Each row of T holds the indices of one triangle, counter-clockwise. Duplicate points are used once. Four co-circular points are split as MATLAB splits them.
Example: delaunay([0 1 0], [0 0 1])   % [1 2 3]
Differences from MATLAB: The rows come in a different order, and start at a different vertex, than MATLAB's; the set of triangles and their orientation match MATLAB. Three-dimensional points are not supported.

## delaunayn
Delaunay triangulation of points given as rows.
Syntax: T = delaunayn(X)
Syntax: T = delaunayn(X, options)
X has one or two columns. Qhull options are accepted and ignored. Three collinear points give the single row [1 2 3], as in MATLAB.
Example: delaunayn([0 0; 1 0; 0 1])   % [1 2 3]
Differences from MATLAB: Points in three or more dimensions are not supported. Rows are ordered by their vertex indices; MATLAB's Qhull order can differ, with the same triangles.

## convhull
Convex hull of two-dimensional points.
Syntax: K = convhull(x, y)
Syntax: K = convhull(P)
Syntax: [K, A] = convhull(..., 'simplify', tf)
K lists the hull points counter-clockwise as a closed column, starting at the smallest index. Points on hull edges are kept unless 'simplify' is true. A is the enclosed area.
Example: convhull([0 1 1 0 0.5], [0 0 1 1 0.5])'   % [1 2 3 4 1]
Differences from MATLAB: Three-dimensional points are not supported.

## convhulln
Convex hull of points given as rows.
Syntax: K = convhulln(X)
Syntax: [K, V] = convhulln(X, options)
For two-column X each row of K is one hull edge, counter-clockwise from the smallest index; points on hull edges are left out. V is the enclosed area. Qhull options are accepted and ignored.
Example: convhulln([0 0; 1 0; 1 1; 0 1])   % [1 2; 2 3; 3 4; 4 1]
Differences from MATLAB: Points in three or more dimensions are not supported.

## dsearchn
Nearest data point to each query point.
Syntax: k = dsearchn(X, XI)
Syntax: [k, d] = dsearchn(X, T, XI)
Syntax: k = dsearchn(X, T, XI, outval)
k holds the row of X nearest to each row of XI and d the distance. With a triangulation T and outval, query points outside the triangulation get outval.
Example: dsearchn([0 0; 1 0; 0 1], [0.9 0.2])   % 2
Differences from MATLAB: Exact ties go to the first point without a triangulation and to the last with one, matching MATLAB.

## tsearchn
Enclosing simplex of each query point.
Syntax: [t, p] = tsearchn(X, T, XI)
t holds the first row of T whose simplex contains each row of XI (NaN outside) and p its barycentric coordinates.
Example: tsearchn([0 0; 1 0; 0 1], [1 2 3], [0.25 0.25])   % 1
Differences from MATLAB: none known for these forms.

## voronoin
Voronoi diagram of two-dimensional points.
Syntax: [V, C] = voronoin(X)
Syntax: [V, C] = voronoin(X, options)
V starts with a point at infinity, then holds the circumcentres of the Delaunay triangles. C{i} lists the vertices of the cell of point i; unbounded cells include 1. Qhull options are accepted and ignored.
Example: [v, c] = voronoin([0 0; 2 0; 1 2; 1 0.7]); c{4}   % [4 2 3]
Differences from MATLAB: Points in three or more dimensions are not supported. Vertex order follows the checked cases; it is not checked against Qhull for unbounded cells with more than two finite vertices.

## quad2d
Numerically integrate over a planar region.
Syntax: q = quad2d(fun, a, b, c, d)
Syntax: [q, errbnd] = quad2d(fun, a, b, c, d, name, value, ...)
fun(x, y) must accept arrays. c and d are numbers or functions of x. Options are 'AbsTol' (1e-10), 'RelTol' (1e-6), 'MaxFunEvals' (2000), 'Singular' (true) and 'FailurePlot' (ignored).
Example: quad2d(@(x, y) x.*y, 0, 1, 0, 1)   % 0.2500
Differences from MATLAB: The last digits of the tiled sum can differ from MATLAB's. Complex integrands are not supported yet. Reaching MaxFunEvals gives no warning, as in MATLAB.

## quadl
Numerically integrate with adaptive Gauss-Lobatto quadrature.
Syntax: q = quadl(fun, a, b)
Syntax: [q, fcnt] = quadl(fun, a, b, tol, trace, p1, p2, ...)
fun must accept a vector and return a vector of the same length. tol defaults to 1e-6; extra arguments are passed to fun.
Example: quadl(@(t) exp(t), 0, 1)   % 1.7183
Differences from MATLAB: The trace argument is ignored. Complex integrands are not supported yet.

## quadv
Vectorized adaptive Simpson quadrature of an array-valued function.
Syntax: Q = quadv(fun, a, b)
Syntax: [Q, fcnt] = quadv(fun, a, b, tol, trace, p1, p2, ...)
fun is called with one scalar at a time and may return an array of any fixed size.
Example: quadv(@(t) [t t.^2], 0, 1)   % [0.5000 0.3333]
Differences from MATLAB: The trace argument is ignored. Complex integrands are not supported yet.

## freqspace
Frequency spacing for frequency responses.
Syntax: f = freqspace(n)
Syntax: f = freqspace(n, 'whole')
Syntax: [f1, f2] = freqspace(n)
Syntax: [x1, y1] = freqspace(n, 'meshgrid')
One output gives n/2+1 points from 0 to 1 ('whole': n points from 0 up to 2). Two outputs give centred two-dimensional frequencies; n may be [rows cols].
Example: freqspace(4)   % [0 0.5000 1.0000]
Differences from MATLAB: none known for these forms.

## iqr
Interquartile range.
Syntax: r = iqr(X)
Syntax: r = iqr(X, dim)
The difference of the 75th and 25th percentiles (midpoint rule, as prctile). NaN values are ignored; iqr([]) is NaN.
Example: iqr([1 2 3 4 5 6 7 8])   % 4
Differences from MATLAB: Two-dimensional arrays only (no N-D arrays).

## eigs
A few eigenvalues and eigenvectors.
Syntax: d = eigs(A)
Syntax: d = eigs(A, k)
Syntax: d = eigs(A, k, sigma)
Syntax: d = eigs(A, k, sigma, opts)
Syntax: [V, D, flag] = eigs(A, B, k, sigma, opts)
Returns the k (default 6) eigenvalues selected by sigma: 'lm' (default), 'sm', 'la', 'sa', 'lr', 'sr', 'li', 'si', 'be' or a number (closest to it). flag is 0 when the eigenvalues were found.
Example: eigs(diag(1:10), 2)   % [10; 9]
Differences from MATLAB: Computed with a dense eigensolver instead of ARPACK, so results agree with MATLAB to about 1e-12 relative, not to every digit. Sparse input is not supported.

## svds
A few singular values and vectors.
Syntax: s = svds(A)
Syntax: s = svds(A, k)
Syntax: s = svds(A, k, sigma)
Syntax: [U, S, V, flag] = svds(A, k, sigma, opts)
Returns the k (default 6) largest singular values, or those nearest sigma (0 gives the smallest).
Example: svds(diag([3 1 2]), 1)   % 3
Differences from MATLAB: Computed densely, so results agree with MATLAB to about 1e-12 relative, not to every digit. Sparse input is not supported.

## condeig
Condition numbers of eigenvalues.
Syntax: c = condeig(A)
Syntax: [V, D, c] = condeig(A)
c(k) is 1/|cos| of the angle between the left and right eigenvectors of the k-th eigenvalue.
Example: condeig([1 2; 0 3])   % [1.4142; 1.4142]
Differences from MATLAB: Some results differ in the last printed digit.

## funm
Evaluate a general matrix function.
Syntax: F = funm(A, fun)
Syntax: F = funm(A, fun, options)
Syntax: [F, exitflag, output] = funm(...)
Uses the Schur-Parlett algorithm. fun is a handle such as @exp, @log, @sin, @cos, @sinh, @cosh, or a function fun(x, k) returning the k-th derivative. options may set TolBlk, TolTay, MaxTerms, MaxSqrt and Ord. MaxTerms must be an integer from 1 to 1000000; nonempty Ord must have one integer label in 1:n per eigenvalue. MaxSqrt is accepted for compatibility and has no effect, because every atomic block is evaluated with a Taylor series. output has fields terms, ind, ord and T.
Example: funm([1 0; 0 2], @exp)   % [2.7183 0; 0 7.3891]
Differences from MATLAB: Some results differ in the last printed digit.

## polyeig
Polynomial eigenvalue problem.
Syntax: e = polyeig(A0, A1, ..., Ap)
Syntax: [X, e] = polyeig(A0, A1, ..., Ap)
Syntax: [X, e, s] = polyeig(A0, A1, ..., Ap)
Solves (A0 + lambda*A1 + ... + lambda^p*Ap)*x = 0 through its companion linearisation. X has unit-norm columns; s holds the eigenvalue condition numbers.
Example: polyeig(-6, 11, -6, 1)   % [1; 2; 3]
Differences from MATLAB: Some results differ in the last printed digits, and eigenvectors can differ by a complex phase.

## qz
Generalized Schur (QZ) factorization.
Syntax: [AA, BB, Q, Z] = qz(A, B)
Syntax: [AA, BB, Q, Z, V, W] = qz(A, B, flag)
Q*A*Z = AA and Q*B*Z = BB with AA, BB upper triangular (quasi-triangular for flag 'real'). flag is 'complex' (default) or 'real'. V and W hold the right and left generalized eigenvectors.
Example: [AA, BB] = qz([1 2; 3 4], eye(2)); x = diag(AA)'   % [-0.3723 5.3723]
Differences from MATLAB: Some results differ in the last printed digit.

## ordschur
Reorder eigenvalues in a Schur factorization.
Syntax: [US, TS] = ordschur(U, T, select)
Syntax: [US, TS] = ordschur(U, T, keyword)
Syntax: [US, TS] = ordschur(U, T, clusters)
Moves the selected eigenvalues to the leading block. keyword is 'lhp', 'rhp', 'udi' or 'udo'; clusters gives a cluster index per eigenvalue (higher first).
Example: [U, T] = schur([1 2; 0 3]); [US, TS] = ordschur(U, T, [0 1]); x = diag(TS)'   % [3 1]
Differences from MATLAB: Results depend on schur, whose output can differ from MATLAB's in sign or the last digit.

## ordeig
Eigenvalues of a quasi-triangular matrix in their diagonal order.
Syntax: e = ordeig(T)
Syntax: e = ordeig(AA, BB)
Reads the eigenvalues from the 1x1 and 2x2 diagonal blocks, in order.
Example: ordeig([1 2; 0 3])'   % [1 3]
Differences from MATLAB: Some results differ in the last printed digit.

## ordqz
Reorder eigenvalues in a QZ factorization.
Syntax: [AAS, BBS, QS, ZS] = ordqz(AA, BB, Q, Z, select)
Syntax: [AAS, BBS, QS, ZS] = ordqz(AA, BB, Q, Z, keyword)
Syntax: [AAS, BBS, QS, ZS] = ordqz(AA, BB, Q, Z, clusters)
Moves the selected generalized eigenvalues to the leading block; keyword is 'lhp', 'rhp', 'udi' or 'udo'.
Example: [AA, BB, Q, Z] = qz([1 2; 0 -3], eye(2)); [AAS, BBS] = ordqz(AA, BB, Q, Z, 'lhp'); x = ordeig(AAS, BBS)'   % [-3 1]
Differences from MATLAB: Some results differ in the last printed digit.

## gsvd
Generalized singular value decomposition.
Syntax: sigma = gsvd(A, B)
Syntax: [U, V, X, C, S] = gsvd(A, B)
Syntax: [U, V, X, C, S] = gsvd(A, B, 0)
A = U*C*X' and B = V*S*X' with C'*C + S'*S = I. One output returns the generalized singular values. The third argument 0 gives the economy-size decomposition.
Example: gsvd([1 2; 3 4], eye(2))'   % [0.3660 5.4650]
Differences from MATLAB: Some results differ in the last printed digit, and the economy form can return a singular-vector pair with the opposite sign.

## cholupdate
Rank-1 update of a Cholesky factorization.
Syntax: R1 = cholupdate(R, x)
Syntax: R1 = cholupdate(R, x, '+')
Syntax: [R1, p] = cholupdate(R, x, '-')
R1'*R1 = R'*R + x*x' ('+', default) or R'*R - x*x' ('-'). With two outputs a failed downdate returns p > 0 instead of an error.
Example: cholupdate(chol([4 2; 2 3]), [1; 0])   % [2.2361 0.8944; 0 1.4832]
Differences from MATLAB: Some results differ in the last printed digit.

## qrupdate
Rank-1 update of a QR factorization.
Syntax: [Q1, R1] = qrupdate(Q, R, u, v)
Q1*R1 = Q*R + u*v' for a full QR factorization [Q, R] = qr(A).
Example: [Q, R] = qr([1 2; 3 4]); [Q1, R1] = qrupdate(Q, R, [1; 0], [0; 1]); x = Q1*R1   % [1 3; 3 4]
Differences from MATLAB: Some results differ in the last printed digit.

## qrinsert
Insert a column or row into a QR factorization.
Syntax: [Q1, R1] = qrinsert(Q, R, j, x)
Syntax: [Q1, R1] = qrinsert(Q, R, j, x, 'col')
Syntax: [Q1, R1] = qrinsert(Q, R, j, x, 'row')
Returns the QR factorization of A with x inserted before column (or row) j.
Example: [Q, R] = qr([1 2; 3 4]); [Q1, R1] = qrinsert(Q, R, 3, [1; 1]); x = Q1*R1   % [1 2 1; 3 4 1]
Differences from MATLAB: Some results differ in the last printed digit.

## qrdelete
Delete a column or row from a QR factorization.
Syntax: [Q1, R1] = qrdelete(Q, R, j)
Syntax: [Q1, R1] = qrdelete(Q, R, j, 'col')
Syntax: [Q1, R1] = qrdelete(Q, R, j, 'row')
Returns the QR factorization of A with column (or row) j removed.
Example: [Q, R] = qr([1 2; 3 4; 5 6]); [Q1, R1] = qrdelete(Q, R, 1); x = Q1*R1   % [2; 4; 6]
Differences from MATLAB: Some results differ in the last printed digit.

## pcg
Preconditioned conjugate gradients for a symmetric positive definite system.
Syntax: x = pcg(A, b)
Syntax: [x, flag, relres, iter, resvec] = pcg(A, b, tol, maxit, M1, M2, x0, ...)
A, M1 and M2 are dense matrices or function handles (afun(x) returns A*x, mfun(x) returns M\x). Defaults: tol 1e-6, maxit min(n,20). flag is 0 (converged), 1 (maxit reached), 2 (ill-conditioned preconditioner), 3 (stagnation) or 4 (breakdown). With one output it prints MATLAB's convergence message.
Example: [x, flag] = pcg([4 1; 1 3], [1; 2])   % x = [0.0909; 0.6364], flag = 0
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## gmres
Restarted generalized minimum residual method.
Syntax: x = gmres(A, b)
Syntax: [x, flag, relres, iter, resvec] = gmres(A, b, restart, tol, maxit, M1, M2, x0, ...)
GMRES with Householder Arnoldi (Saad and Schultz, 1986), left preconditioned: relres and resvec measure M\(b - A*x) relative to M\b. With restart, iter is [outer inner] and maxit counts outer iterations. Defaults: no restart, tol 1e-6, maxit min(n,10).
Example: [x, flag, relres, iter] = gmres([4 1; 2 5], [1; 0])   % iter = [1 2]
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## bicg
Biconjugate gradients.
Syntax: x = bicg(A, b)
Syntax: [x, flag, relres, iter, resvec] = bicg(A, b, tol, maxit, M1, M2, x0, ...)
A function handle is called as afun(x,'notransp') for A*x and afun(x,'transp') for A'*x; preconditioner functions likewise.
Example: [x, flag] = bicg([4 1; 2 5], [1; 2])   % flag = 0
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## bicgstab
Biconjugate gradients stabilized.
Syntax: x = bicgstab(A, b)
Syntax: [x, flag, relres, iter, resvec] = bicgstab(A, b, tol, maxit, M1, M2, x0, ...)
iter can be a half iteration (for example 2.5); resvec has two entries per iteration.
Example: [x, flag, relres, iter] = bicgstab([4 1 0; 2 5 1; 0 1 3], [1; 2; 3])   % iter = 2.5
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## bicgstabl
Biconjugate gradients stabilized (l), with l = 2.
Syntax: x = bicgstabl(A, b)
Syntax: [x, flag, relres, iter, resvec] = bicgstabl(A, b, tol, maxit, M1, M2, x0, ...)
iter counts quarter iterations (for example 1.25); resvec has four entries per iteration.
Example: [x, flag, relres, iter] = bicgstabl([4 1 0; 2 5 1; 0 1 3], [1; 2; 3])   % iter = 1.25
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## cgs
Conjugate gradients squared.
Syntax: x = cgs(A, b)
Syntax: [x, flag, relres, iter, resvec] = cgs(A, b, tol, maxit, M1, M2, x0, ...)
Example: [x, flag] = cgs([4 1; 2 5], [1; 2])   % flag = 0
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## minres
Minimum residual method for a symmetric (possibly indefinite) system.
Syntax: x = minres(A, b)
Syntax: [x, flag, relres, iter, resvec, resveccg] = minres(A, b, tol, maxit, M1, M2, x0, ...)
The preconditioner M = M1*M2 must be symmetric positive definite (flag 5 otherwise).
Example: [x, flag] = minres([2 1 0; 1 -3 1; 0 1 1], [1; 2; 3])   % x = [1/3; 1/3; 8/3]
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## lsqr
Least squares QR method for a rectangular or square system.
Syntax: x = lsqr(A, b)
Syntax: [x, flag, relres, iter, resvec, lsvec] = lsqr(A, b, tol, maxit, M1, M2, x0, ...)
Solves min norm(b - A*x). A function handle is called as afun(x,'notransp') and afun(x,'transp'). Defaults: tol 1e-6, maxit min([size(A) 20]). lsvec holds the scaled estimates of norm(A'*r).
Example: [x, flag] = lsqr([1 2; 3 4; 5 6], [1; 2; 4])   % x = [0.6667; 0.0833]
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## qmr
Quasi-minimal residual method.
Syntax: x = qmr(A, b)
Syntax: [x, flag, relres, iter, resvec] = qmr(A, b, tol, maxit, M1, M2, x0, ...)
A function handle is called as afun(x,'notransp') and afun(x,'transp').
Example: [x, flag] = qmr([4 1; 2 5], [1; 2])   % flag = 0
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## symmlq
Symmetric LQ method for a symmetric (possibly indefinite) system.
Syntax: x = symmlq(A, b)
Syntax: [x, flag, relres, iter, resvec, resveccg] = symmlq(A, b, tol, maxit, M1, M2, x0, ...)
Returns the conjugate-gradient point when it has the smaller residual. The preconditioner must be symmetric positive definite (flag 5 otherwise).
Example: [x, flag] = symmlq([2 1 0; 1 -3 1; 0 1 1], [1; 2; 3])   % flag = 0
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.

## tfqmr
Transpose-free quasi-minimal residual method.
Syntax: x = tfqmr(A, b)
Syntax: [x, flag, relres, iter, resvec] = tfqmr(A, b, tol, maxit, M1, M2, x0, ...)
Each iteration has two half steps; resvec has an entry for each.
Example: [x, flag] = tfqmr([4 1; 2 5], [1; 2])   % flag = 0
Differences from MATLAB: Real dense matrices and real-valued function handles only; complex and sparse systems are unsupported. Singular dense preconditioners return flag 2 without MATLAB's singular-matrix warning.
