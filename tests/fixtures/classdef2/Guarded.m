classdef Guarded < handle
  % delete is private: only the class's own methods may destroy it.
  properties
    v = 1
  end
  methods
    function dispose(obj)
      delete(obj);
    end
  end
  methods (Access = private)
    function delete(obj)
      fprintf('deleting Guarded\n');
    end
  end
end
