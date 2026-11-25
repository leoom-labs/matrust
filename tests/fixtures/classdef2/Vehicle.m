classdef Vehicle
  % An abstract class: wheels and drive have signatures only.
  properties
    maker = 'none'
  end
  methods (Abstract)
    n = wheels(obj)
    [ok, msg] = drive(obj, distance)
  end
  methods
    function r = summary(obj)
      r = sprintf('%s on %d wheels', obj.maker, obj.wheels());
    end
  end
end
