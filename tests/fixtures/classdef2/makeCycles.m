function n = makeCycles(count)
% Builds count two-node handle cycles inside a function and drops each.
n = 0;
for i = 1:count
  a = Node;
  b = Node;
  a.next = b;
  b.next = a;
  n = n + 1;
end
end
