classdef Pt
  % Point with public, private and Constant properties.
  properties
    x = 1
    label = 'a'
    data = [1 2 3]
    info = struct('a', 1, 'b', 'x')
  end
  properties (Access = private)
    k = 7
  end
  properties (SetAccess = private)
    ro = 5
  end
  properties (Constant)
    C = 3 * 2
  end
  methods
    function obj = Pt(x)
      if nargin > 0
        obj.x = x;
      end
    end
    function r = scaled(obj, s)
      r = obj.x * s + obj.k;
    end
    function [a, b, c] = several(obj, varargin)
      a = obj.x;
      b = nargin;
      c = nargout;
    end
    function obj = setk(obj, v)
      obj.k = v;
    end
    function obj = setro(obj, v)
      obj.ro = v;
    end
    function forget(obj)
      obj.x = -1;
    end
    function r = viaSecret(obj)
      r = secret(obj) + twice(obj.x);
    end
  end
  methods (Access = private)
    function r = secret(obj)
      r = obj.k * 10;
    end
  end
  methods (Static)
    function p = make(v)
      p = Pt(v * Pt.C);
    end
  end
end
function y = twice(v)
  y = 2 * v;
end
