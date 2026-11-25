function v = attachVia(count)
% Hangs a new Node on the caller's h, then removes every name for h: only
% the caller's unfinished expression still holds it. The cycles built after
% that make the collection run between this function's statements; the new
% Node must survive it.
h = evalin('caller', 'h');
h.next = Node;
h.next.value = 7;
evalin('caller', 'clear h');
clear h
for i = 1:count
  a = Node;
  b = Node;
  a.next = b;
  b.next = a;
end
v = count;
end
