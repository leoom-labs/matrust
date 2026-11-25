classdef Badge
  % SetAccess = immutable: only the constructor sets id.
  properties (SetAccess = immutable)
    id = 0
  end
  properties
    owner = ''
  end
  methods
    function obj = Badge(id)
      if nargin > 0
        obj.id = id;
      end
      obj = obj.stamp();
    end
    function obj = stamp(obj)
      obj.owner = sprintf('badge %d', obj.id);
    end
    function obj = rename(obj, id)
      obj.id = id;
    end
  end
end
