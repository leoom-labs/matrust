% Explicit marker symbols and sizes survive editable figure saves.
x = [1 2 3 4];
plot(x, [4 4 4 4], 'b-', 'Marker', 'square', 'MarkerSize', 10, 'LineWidth', 1.5);
hold on;
plot(x, [3 3 3 3], 'r-', 'Marker', 'x', 'MarkerSize', 12, 'LineWidth', 1.5);
plot(x, [2 2 2 2], 'g-', 'Marker', '^', 'MarkerSize', 14, 'LineWidth', 1.5);
plot(x, [1 1 1 1], 'm-', 'Marker', 'diamond', 'MarkerSize', 16, 'LineWidth', 1.5);
plot(x, [0 0 0 0], 'k-', 'Marker', '.', 'MarkerSize', 18, 'LineWidth', 1.5);
legend('Square 10 pt', 'Cross 12 pt', 'Triangle 14 pt', 'Diamond 16 pt', 'Point 18 pt');
title('Marker shapes and sizes');
xlabel('Sample'); ylabel('Series');
axis([0.5 4.5 -0.5 5.5]);
hold off;
