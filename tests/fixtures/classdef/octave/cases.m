% Cases for tests/classdef_part1.rs: printed with fprintf only so that the
% GNU Octave 10.3 output (cases-expected.txt) compares line by line.
p = Plain;
fprintf('plain a empty: %d\n', isempty(p.a));
fprintf('plain b: %s\n', p.b);
q = Pt;
fprintf('default x: %g\n', q.x);
p = Pt(3);
fprintf('ctor x: %g\n', p.x);
fprintf('label: %s\n', p.label);
b = p;
b.x = 10;
fprintf('copy keeps original: %g %g\n', p.x, b.x);
forget(p);
fprintf('method without return: %g\n', p.x);
p.forget();
fprintf('dot method without return: %g\n', p.x);
fprintf('dot call: %g\n', p.scaled(2));
fprintf('function call: %g\n', scaled(p, 3));
p2 = p.setk(5);
fprintf('returned copy: %g %g\n', p.scaled(1), p2.scaled(1));
fprintf('static: %g\n', Pt.make(2).x);
m = Pt.make(1);
fprintf('static var: %g\n', m.x);
fprintf('constant: %g\n', Pt.C);
fprintf('constant via object: %g\n', p.C);
[a1, a2, a3] = p.several('u', 'v');
fprintf('several dot: %g %g %g\n', a1, a2, a3);
[a1, a2] = several(p);
fprintf('several call: %g %g\n', a1, a2);
p.data(2) = 20;
fprintf('indexed property: %s\n', mat2str(p.data));
fprintf('indexed read: %g %g\n', p.data(2), p.data(end));
p.info.b = 'changed';
fprintf('nested field: %s %g\n', p.info.b, p.info.a);
p.info.c = 3;
fprintf('new nested field: %g\n', p.info.c);
fprintf('private via method: %g\n', p.viaSecret());
fprintf('class: %s\n', class(p));
fprintf('isa: %d %d\n', isa(p, 'Pt'), isa(p, 'Plain'));
fprintf('isobject: %d %d\n', isobject(p), isobject(struct('x', 1)));
fprintf('isstruct: %d\n', isstruct(p));
f = fieldnames(p);
fprintf('fieldnames: %s\n', strjoin(f', ','));
f = properties(p);
fprintf('properties: %s\n', strjoin(f', ','));
f = properties('Pt');
fprintf('properties by name: %s\n', strjoin(f', ','));
f = methods(p);
fprintf('methods: %s\n', strjoin(f', ','));
fprintf('isprop: %d %d %d\n', isprop(p, 'x'), isprop(p, 'k'), isprop(p, 'nope'));
fprintf('ismethod: %d %d %d\n', ismethod(p, 'scaled'), ismethod(p, 'secret'), ismethod(p, 'nope'));
fprintf('isequal: %d %d %d\n', isequal(Pt(3), Pt(3)), isequal(Pt(3), Pt(4)), isequal(Pt(1), Plain));
s = struct(p);
fprintf('struct fields: %s\n', strjoin(fieldnames(s)', ','));
fprintf('struct k: %g\n', s.k);
try
  v = p.k;
  fprintf('private read: no error\n');
catch
  fprintf('private read: error\n');
end
try
  p.k = 1;
  fprintf('private write: no error\n');
catch
  fprintf('private write: error\n');
end
try
  p.ro = 1;
  fprintf('SetAccess private write: no error\n');
catch
  fprintf('SetAccess private write: error\n');
end
p3 = p.setro(9);
fprintf('SetAccess private via method: %g\n', p3.ro);
try
  secret(p);
  fprintf('private method: no error\n');
catch
  fprintf('private method: error\n');
end
try
  p.C = 1;
  fprintf('constant write: no error\n');
catch
  fprintf('constant write: error\n');
end
try
  v = p.nope;
  fprintf('unknown read: no error\n');
catch
  fprintf('unknown read: error\n');
end
try
  p.nope = 1;
  fprintf('unknown write: no error\n');
catch
  fprintf('unknown write: error\n');
end
t1 = Tally;
t2 = Tally;
fprintf('default once: %g %g\n', t1.first, t2.first);
disp(t1);
t1.disp();
