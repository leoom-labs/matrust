% Local functions keep their temporary variables out of your workspace.
t = linspace(0, 2, 2000);
signal = sin(2*pi*3*t) + 0.25*cos(2*pi*180*t);
b = fir1(48, 0.12);
[clean, final_state] = filter_chunks(b, signal, 256);
reference = filter(b, 1, signal);
error = norm(clean-reference)
show_result(t, signal, clean);

function [y, state] = filter_chunks(b, x, chunk)
    if nargin < 3
        chunk = 256;
    end
    y = zeros(1, numel(x));
    state = [];
    for first = 1:chunk:numel(x)
        last = first+chunk-1;
        if last > numel(x)
            last = numel(x);
        end
        [part, state] = filter(b, 1, x(first:last), state);
        y(first:last) = part;
    end
end

function show_result(t, original, filtered)
    plot(t, original, 'b:', t, filtered, 'r-');
    title('Reusable streaming filter functions');
    xlabel('Time (seconds)');
    ylabel('Amplitude');
    legend('Original', 'Filtered');
    grid on;
end
