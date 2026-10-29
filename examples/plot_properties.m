% MATLAB-style trailing name/value properties affect all series in this call.
x = 0:0.1:6;
plot(x, sin(x), 'bo-', x, cos(x), 'r--', 'LineWidth', 2);
title('Script-defined line widths and styles');
xlabel('Time'); ylabel('Amplitude');
legend('Sine', 'Cosine');
hold on;
plot(x, 0.5*sin(x), 'Color', [0.1 0.55 0.25], 'LineWidth', 4, 'DisplayName', 'Half amplitude');
hold off;
