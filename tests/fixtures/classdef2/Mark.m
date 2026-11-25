classdef Mark
  % Base of Quiet and Loud: counts how often its constructor runs on the
  % object and how many arguments it got.
  properties
    runs = 0
    args = -1
  end
  methods
    function obj = Mark(varargin)
      obj.runs = obj.runs + 1;
      obj.args = nargin;
    end
  end
end
