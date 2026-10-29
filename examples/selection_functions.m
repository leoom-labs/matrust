% Select the lines between the markers and press F9.
% The selected call uses the local function below without running this line:
unselected = missing;
% Begin selection
t = linspace(0,2,2000);
y = make_signal(t);
plot(t,y);
title('F9 selection using a local function');
xlabel('Time (s)');
ylabel('Amplitude');
% End selection

function y = make_signal(t)
y = exp(-t) .* sin(2*pi*5*t);
end
