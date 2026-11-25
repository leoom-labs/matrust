% Generator of octave.txt: bash tool/ref-octave.sh tests/fixtures/sparse_precond/cases.m > octave.txt (Octave 10.3, 2026-11-04).
m = 5; e = ones(m,1);
T = spdiags([-e 2*e -e],-1:1,m,m); I = speye(m);
P = kron(I,T) + kron(T,I);
T1 = spdiags([-1.3*e 4*e -0.7*e],-1:1,m,m); T2 = spdiags([-0.6*e 0*e -1.1*e],-1:1,m,m);
A = kron(I,T1) + kron(T2,I);
p = [3 1 4 15 9 2 6 25 5 8 7 10 11 12 13 14 16 17 18 19 20 21 22 23 24];
B = A(p,:);
function dump(tag, M)
  [i,j,v] = find(M);
  fprintf('%s %d\n', tag, numel(v));
  for k = 1:numel(v)
    fprintf('%d %d %.17g\n', i(k), j(k), v(k));
  end
end
dump('ic0', ichol(P));
o.michol = 'on'; dump('mic0', ichol(P, o));
o = struct(); o.type = 'ict'; o.droptol = 0.05; dump('ict', ichol(P, o));
o.michol = 'on'; dump('mict', ichol(P, o));
o = struct(); o.diagcomp = 0.1; o.shape = 'upper'; dump('icup', ichol(P, o));
[L,U] = ilu(A); dump('ilu0L', L); dump('ilu0U', U);
s.milu = 'row'; dump('ilu0row', ilu(A, s));
s.milu = 'col'; dump('ilu0col', ilu(A, s));
s = struct(); s.type = 'crout'; s.droptol = 0.05; [L,U] = ilu(A, s); dump('crL', L); dump('crU', U);
s.droptol = 0; dump('cr0', ilu(A, s));
s.droptol = 0.05; s.milu = 'row'; dump('crrow', ilu(A, s));
s.milu = 'col'; dump('crcol', ilu(A, s));
s = struct(); s.type = 'ilutp'; s.droptol = 0.05; [L,U,Pm] = ilu(B, s); dump('tpL', L); dump('tpU', U); dump('tpP', Pm);
s.thresh = 0.5; [L,U] = ilu(B, s); dump('tp5L', L); dump('tp5U', U);
s.droptol = 0; s.thresh = 1; [L,U,Pm] = ilu(B, s); dump('tp0L', L); dump('tp0U', U);
s.droptol = 0.05; s.milu = 'col'; dump('tpcol', ilu(B, s));
