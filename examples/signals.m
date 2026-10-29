% Sample a damped wave and plot it.
t = linspace(0, 12, 1200);
y = exp(-0.15 .* t) .* sin(3 .* t);
plot(t, y);
title('Damped oscillation');
xlabel('Time (s)');
ylabel('Amplitude');
peak = max(y)
energy = sum(y.^2)
