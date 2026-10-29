% Error-navigation example: the helper intentionally refers to a missing name.
% Run this script, then choose Go to error to open the failing helper line.
t = linspace(0,1,1000);
x = sin(2*pi*5*t);
plot(t,x);
title('Signal before the helper error');
y = normalize_signal(x);
