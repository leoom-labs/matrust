classdef Counter < handle
  % A handle class: copies share one object.
  properties
    count = 0
    label = 'c'
  end
  methods
    function obj = Counter(n)
      if nargin > 0
        obj.count = n;
      end
    end
    function bump(obj, d)
      if nargin < 2
        d = 1;
      end
      obj.count = obj.count + d;
    end
    function delete(obj)
      fprintf('deleting Counter %g\n', obj.count);
    end
  end
end
