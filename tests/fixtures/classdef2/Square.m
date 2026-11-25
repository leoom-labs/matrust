classdef Square < Rect
  methods
    function obj = Square(s)
      obj = obj@Rect(s, s);
      obj.name = 'square';
    end
    function r = tag(obj)
      s = tag@Rect(obj);
      r = ['Square>' s];
    end
  end
end
