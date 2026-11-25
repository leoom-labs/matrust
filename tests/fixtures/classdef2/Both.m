classdef Both < Left & Right
  % who is defined here; common comes from both superclasses (ambiguous).
  methods
    function r = who(obj)
      a = who@Left(obj);
      b = who@Right(obj);
      r = ['Both:' a '+' b];
    end
  end
end
