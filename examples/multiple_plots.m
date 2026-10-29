% Multiple editable series and figures.
figure(1);
x = linspace(0,2*pi,160);
plot(x,sin(x),'-',x,cos(x),'--');
hold on;
scatter([0 pi/2 pi 3*pi/2 2*pi],[0 1 0 -1 0]);
legend('sin(x)','cos(x)','Samples');
title('Signals and sampled measurements');
xlabel('Time (s)');ylabel('Amplitude');
axis([0 2*pi -1.3 1.3]);grid on;
figure(2);
bar([1 2 3 4],[2 3;4 2;3 5;6 4]);
legend('Measured','Reference');
title('Grouped measurements');xlabel('Experiment');ylabel('Value');
figure(1);
