classdef Left < Root
  properties
    left = 'L'
  end
  methods
    function obj = Left()
      fprintf('Left constructor\n');
    end
    function r = side(obj)
      r = 'Left.side';
    end
    function r = who(obj)
      r = 'Left';
    end
    function r = common(obj)
      r = 'Left.common';
    end
  end
end
