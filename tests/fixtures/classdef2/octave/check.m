% Octave 10.3 confirmation of classdef part 2 behaviour (handle classes,
% inheritance, get/set methods). Run from the repository root:
%   tool/ref-octave.sh tool/refs/classdef2/check.m > tests/fixtures/classdef2/octave/check-expected.txt
% tests/classdef_part2.rs runs the same script (without the addpath line)
% and compares the output. Only fprintf output, so both print alike.
addpath(fullfile(fileparts(mfilename('fullpath')), '..'));

% 1. Handle copies alias one object; a method changes it without returning it.
a = Acc(1); b = a; b.total = 5; a.add(2); add(b, 3);
fprintf('1 handle: a.total=%g b.total=%g\n', a.total, b.total);

% 2. isequal compares handle property values.
x = Box; y = Box; z = x;
fprintf('2 isequal: same=%d equal_values=%d\n', isequal(x, z), isequal(x, y));
y.v = 2;
fprintf('2 isequal: after_change=%d\n', isequal(x, y));

% 3. Three-level chain: obj@Super constructor arguments, method@Super, isa.
q = Square(3);
fprintf('3 chain: area=%g name=%s tag=%s\n', q.area(), q.name, q.tag());
fprintf('3 chain: isa Square=%d Rect=%d Shape=%d class=%s\n', ...
  isa(q, 'Square'), isa(q, 'Rect'), isa(q, 'Shape'), class(q));
fprintf('3 chain: %s\n', q.describe());

% 4. Protected: a subclass method reads it, outside code cannot.
r = Rect(2, 5);
fprintf('4 protected: peek=%g\n', r.peek());
try
  r.secret;
  fprintf('4 protected: outside read allowed\n');
catch
  fprintf('4 protected: outside read rejected\n');
end

% 5. set/get methods: constructor and other methods call set.celsius;
%    Dependent fahrenheit/kelvin are computed.
t = Temp(100);
fprintf('5 getset: f=%g k=%g sets=%g\n', t.fahrenheit, t.kelvin, t.sets);
t.fahrenheit = 32;
fprintf('5 getset: c=%g sets=%g\n', t.celsius, t.sets);
t = t.warm(5);
fprintf('5 getset: c=%g sets=%g\n', t.celsius, t.sets);
try
  t.celsius = -300;
catch err
  fprintf('5 getset: rejected: %s\n', err.message);
end

% 6. Recursion guard: inside get.value / set.value the property is storage.
g = Logged;
v1 = g.value; v2 = g.value;
g.value = 5;
fprintf('6 guard: v1=%g value=%g reads=%g\n', v1, g.value, g.reads);

% 7. Implicit superclass constructors run left to right.
m = Both;
fprintf('7 multiple: side=%s only=%s root=%s who=%s\n', m.side(), m.onlyRight(), m.rootName(), m.who());
