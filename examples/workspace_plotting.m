% Select y in Workspace, then Plots > Plot workspace data.
% Choose t as X data, then create a new figure or open its script.
t = linspace(0,1,2000);
y = [sin(2*pi*5*t)' cos(2*pi*9*t)'];
plot(t,y);
xlabel('Time (s)');
ylabel('Amplitude');
title('Original signals');
legend('5 Hz','9 Hz');
