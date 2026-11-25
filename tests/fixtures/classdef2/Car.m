classdef Car < Vehicle
  methods
    function obj = Car(maker)
      obj.maker = maker;
    end
    function n = wheels(obj)
      n = 4;
    end
    function [ok, msg] = drive(obj, distance)
      ok = distance < 100;
      msg = sprintf('%s drove %g', obj.maker, distance);
    end
  end
end
