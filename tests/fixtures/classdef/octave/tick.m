function y = tick()
  % Counts its calls: a class default calling it runs once per run.
  persistent n
  if isempty(n)
    n = 0;
  end
  n = n + 1;
  y = n;
end
