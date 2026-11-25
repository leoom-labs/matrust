classdef Quiet < Mark
  % Its constructor names obj@Mark only in a comment and in text, so
  % Mark's constructor still runs implicitly, with no arguments.
  properties
    note = ''
  end
  methods
    function obj = Quiet()
      % obj = obj@Mark(5);
      obj.note = 'obj@Mark(5)';
      label = "obj = obj@Mark(6)";
    end
  end
end
