% Tiled axes keep independent data and views; save as MFIG to edit later.
tiledlayout(2,2);
sgtitle('Signal comparison');
t=linspace(0,1,200);
nexttile;
plot(t,sin(2*3.141592653589793*3*t));
title('Input'); xlabel('Time (s)'); ylabel('Amplitude');
nexttile;
plot(t,cos(2*3.141592653589793*3*t));
title('Quadrature'); xlabel('Time (s)'); ylabel('Amplitude');
nexttile([1 2]);
n=0:24; x=sin(n/3); y=filter([0.25 0.5 0.25],1,x);
stairs(n,x); hold on; stem(n,y); hold off;
title('Discrete filter'); xlabel('Sample'); ylabel('Amplitude');
legend('Input','Filtered');
% Select an existing tile without replacing its samples.
nexttile(1);
