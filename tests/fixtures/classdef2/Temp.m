classdef Temp
  % get/set methods, Dependent and Hidden properties.
  properties
    celsius = 0
  end
  properties (Dependent)
    fahrenheit
    kelvin
  end
  properties (Hidden)
    sets = 0
  end
  methods
    function obj = Temp(c)
      if nargin > 0
        obj.celsius = c;
      end
    end
    function v = get.fahrenheit(obj)
      v = obj.celsius * 9 / 5 + 32;
    end
    function obj = set.fahrenheit(obj, f)
      obj.celsius = (f - 32) * 5 / 9;
    end
    function v = get.kelvin(obj)
      v = obj.celsius + 273.15;
    end
    function obj = set.celsius(obj, c)
      if c < -273.15
        error('Temp:range', 'below absolute zero');
      end
      obj.celsius = c;
      obj.sets = obj.sets + 1;
    end
    function obj = warm(obj, d)
      obj.celsius = obj.celsius + d;
    end
  end
end
