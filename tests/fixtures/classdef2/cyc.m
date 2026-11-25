function cyc()
% Builds one two-node handle cycle and drops it on return.
a = Node;
b = Node;
a.next = b;
b.next = a;
end
