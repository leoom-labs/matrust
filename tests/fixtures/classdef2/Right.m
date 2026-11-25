classdef Right < Root
  properties
    right = 'R'
  end
  methods
    function obj = Right()
      fprintf('Right constructor\n');
    end
    function r = who(obj)
      r = 'Right';
    end
    function r = onlyRight(obj)
      r = 'Right.onlyRight';
    end
    function r = common(obj)
      r = 'Right.common';
    end
  end
end
