classdef Rect < Shape
  properties
    w = 1
    h = 1
  end
  methods
    function obj = Rect(w, h)
      obj = obj@Shape('rect');
      if nargin > 0
        obj.w = w;
        obj.h = h;
      end
    end
    function a = area(obj)
      a = obj.w * obj.h;
    end
    function r = tag(obj)
      s = tag@Shape(obj);
      r = ['Rect>' s];
    end
    function s = peek(obj)
      s = obj.secret;
    end
  end
end
