classdef Bike < Vehicle
  % Implements wheels only: still abstract.
  methods
    function n = wheels(obj)
      n = 2;
    end
  end
end
