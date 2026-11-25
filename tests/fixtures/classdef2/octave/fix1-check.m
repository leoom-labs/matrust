% Octave 10.3 confirmation for fix round 1 (run from the repository root):
%   tool/ref-octave.sh tool/refs/classdef2/fix1-check.m > tests/fixtures/classdef2/octave/fix1-check-expected.txt
% (Octave then prints "error: ignoring const execution_exception& while
% preparing to exit" on stderr; the output is complete.)
% 1. obj@Mark only in a comment and in text: Mark's constructor runs
%    implicitly with no arguments. An explicit call runs it once with its
%    argument.
% 2. Two-node handle cycles built and dropped in a loop, at top level and in
%    a function, then a live list survives.
addpath(fullfile(fileparts(mfilename('fullpath')), '..'));
q = Quiet;
fprintf('1 quiet: runs=%g args=%g note=%s\n', q.runs, q.args, q.note);
d = Loud;
fprintf('1 loud: runs=%g args=%g\n', d.runs, d.args);
for i = 1:2000
  a = Node; b = Node; a.next = b; b.next = a;
end
clear a b
n = makeCycles(2000);
list = Node; list.next = Node; list.next.value = 3;
fprintf('2 cycles: n=%g list=%g\n', n, list.next.value);
