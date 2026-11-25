classdef Loud < Mark
  % Calls Mark's constructor explicitly: it runs once, with the argument.
  methods
    function obj = Loud()
      obj = obj@Mark(5);
    end
  end
end
