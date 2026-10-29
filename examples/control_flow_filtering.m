% Compare FIR orders while filtering bounded chunks with retained state.
t = linspace(0, 2, 2000);
signal = sin(2*pi*3*t) + 0.25*cos(2*pi*180*t);
plot(t, signal, 'b-');
hold on;
filters_run = 0;
chunks_run = 0;
for order = [8 32 64]
    b = fir1(order, 0.12);
    state = [];
    clean = zeros(1, numel(signal));
    for first = 1:256:numel(signal)
        last = first + 255;
        if last > numel(signal)
            last = numel(signal);
        end
        [piece, state] = filter(b, 1, signal(first:last), state);
        clean(first:last) = piece;
        chunks_run = chunks_run + 1;
    end
    if order == 8
        plot(t, clean, 'g-');
    elseif order == 32
        plot(t, clean, 'r-');
    else
        plot(t, clean, 'k-');
    end
    filters_run = filters_run + 1;
end
hold off;
title('Compare FIR orders with streaming script loops');
xlabel('Time (seconds)');
ylabel('Amplitude');
legend('Original', 'FIR 8', 'FIR 32', 'FIR 64');
grid on;
filters_run
chunks_run
