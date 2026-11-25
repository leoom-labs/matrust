classdef NoGet
  properties (Dependent)
    d
  end
  methods
    function obj = set.d(obj, v)
      fprintf('set.d %g\n', v);
    end
  end
end
