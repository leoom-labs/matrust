classdef Tally
  properties
    first = 100 + tick()
    second = 2
  end
  methods
    function disp(obj)
      fprintf('<Tally first=%g second=%g>\n', obj.first, obj.second);
    end
  end
end
