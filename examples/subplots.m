% Compare signals and their samples in one editable figure.
clf;
t = linspace(0,2*pi,200);
subplot(2,2,1);
plot(t,sin(t),'b-');
title('Sine'); xlabel('Time'); ylabel('Amplitude');
subplot(2,2,2);
plot(t,cos(t),'r--');
title('Cosine'); xlabel('Time'); ylabel('Amplitude');
subplot(2,2,[3 4]);
n = 0:24;
x = sin(0.25*pi*n);
stairs(n,x,'b-'); hold on;
stem(n,filter([0.25 0.5 0.25],1,x),'r-','filled');
title('Discrete input and filtered samples');
xlabel('Sample'); ylabel('Amplitude');
legend('Input','Filtered');
