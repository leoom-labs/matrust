classdef Shape
  % The base of Rect and Square (Octave confirms this chain, so it is not
  % abstract: Octave 10.3 rejects abstract method signatures).
  properties
    name = 'shape'
  end
  properties (Access = protected)
    secret = 42
  end
  methods
    function obj = Shape(n)
      if nargin > 0
        obj.name = n;
      end
    end
    function d = describe(obj)
      d = sprintf('%s with area %g', obj.name, obj.area());
    end
    function r = tag(obj)
      r = 'Shape';
    end
    function a = area(obj)
      a = 0;
    end
  end
end
