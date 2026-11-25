classdef Holder < handle
  % Holds one array set by its constructor.
  properties
    data = []
  end
  methods
    function obj = Holder(d)
      if nargin > 0
        obj.data = d;
      end
    end
  end
end
