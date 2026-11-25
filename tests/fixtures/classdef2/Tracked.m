classdef Tracked < Counter
  % A subclass of a handle class is a handle class.
  properties
    history = []
  end
  methods
    function bump(obj, d)
      bump@Counter(obj, d);
      obj.history(end + 1) = obj.count;
    end
    function delete(obj)
      fprintf('deleting Tracked\n');
    end
  end
end
