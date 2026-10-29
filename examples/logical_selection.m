% Select and clean measurements without changing the original recording.
t = linspace(0, 2, 2000);
signal = sin(2*pi*3*t) + 0.15*cos(2*pi*19*t);
selected = (signal > 0.65) & (t > 0.2) & (t < 1.8);
clean = signal;
clean(selected) = NaN;
plot(t, signal, 'b-', t, clean, 'g-', t(selected), signal(selected), 'ro');
title('Select measurements with a logical mask');
xlabel('Time (seconds)');
ylabel('Amplitude');
legend('Original', 'Retained', 'Selected');
grid on;
selected_count = nnz(selected)
