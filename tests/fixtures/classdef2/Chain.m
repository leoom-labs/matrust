classdef Chain < handle
  % A doubly linked list: push links the new node both ways, so every two
  % nodes form a reference cycle; reset drops them all.
  properties
    head = []
    count = 0
  end
  methods
    function push(obj, v)
      n = Link;
      n.value = v;
      if ~isempty(obj.head)
        n.next = obj.head;
        obj.head.prev = n;
      end
      obj.head = n;
      obj.count = obj.count + 1;
    end
    function reset(obj)
      obj.head = [];
    end
  end
end
