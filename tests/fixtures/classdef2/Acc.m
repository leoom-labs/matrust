classdef Acc < handle
  % A handle class without a delete method (Octave reference cases).
  properties
    total = 0
  end
  methods
    function obj = Acc(start)
      if nargin > 0
        obj.total = start;
      end
    end
    function add(obj, v)
      obj.total = obj.total + v;
    end
  end
end
