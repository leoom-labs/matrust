disp('one');
stop_here(5);
disp('never');
function stop_here(code)
  try
    quit(code);
  catch
    disp('caught');
  end
  disp('after catch');
end
