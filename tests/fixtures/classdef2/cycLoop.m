function n = cycLoop(count)
% Calls cyc count times: the cycles each call drops must be collected
% between this function's own statements.
n = 0;
for i = 1:count
  cyc();
  n = n + 1;
end
end
