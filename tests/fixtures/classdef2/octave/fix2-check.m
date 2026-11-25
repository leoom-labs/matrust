% Octave 10.3 confirmation for fix round 2 (run from the repository root):
%   tool/ref-octave.sh tool/refs/classdef2/fix2-check.m > tests/fixtures/classdef2/octave/fix2-check-expected.txt
% 1. Cycles built and dropped inside called functions and methods: a
%    top-level loop calling cyc, cycLoop calling cyc, a doubly linked list
%    linked in a push method and reset.
% 2. Replaced handle objects holding large arrays do not use up memory.
% 3. A private delete method: delete(g) and g.delete() from outside error.
%    Not compared: delete(obj) inside the class's own method. Octave runs
%    the delete method but leaves the object usable (it never invalidates
%    handle objects) and runs delete again at exit; the documentation
%    (delete invalidates every copy) wins, see tests/classdef_part2.rs.
addpath(fullfile(fileparts(mfilename('fullpath')), '..'));
for i = 1:2000
  cyc();
end
n = cycLoop(2000);
c = Chain;
for i = 1:2000
  c.push(i); c.push(-i); c.reset();
end
c.push(5); c.push(6);
fprintf('1 cycles: i=%g n=%g count=%g back=%g\n', i, n, c.count, c.head.next.prev.value);
for k = 1:30
  x = Holder(rand(1200));
end
fprintf('2 holders: k=%g size=%g\n', k, size(x.data, 1));
g = Guarded;
try
  delete(g);
  fprintf('3 delete(g): ran\n');
catch
  fprintf('3 delete(g): error\n');
end
try
  g.delete();
  fprintf('3 g.delete(): ran\n');
catch
  fprintf('3 g.delete(): error\n');
end
fprintf('3 still usable: v=%g\n', g.v);
