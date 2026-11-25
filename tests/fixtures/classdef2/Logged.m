classdef Logged < handle
  % A handle class whose get method counts reads and whose set method
  % (no output) doubles the value.
  properties
    value = 1
    reads = 0
  end
  methods
    function v = get.value(obj)
      obj.reads = obj.reads + 1;
      v = obj.value;
    end
    function set.value(obj, v)
      obj.value = v * 2;
    end
  end
end
