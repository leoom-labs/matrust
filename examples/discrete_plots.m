% Compare the sample-and-hold view with discrete signal samples.
n = 0:40;
x = sin(0.24*pi*n) + 0.25*sin(0.8*pi*n);
y = filter([0.25 0.5 0.25],1,x);
stairs(n,x,'b-');
hold on;
stem(n,y,'r-','filled');
title('Discrete signal and filtered samples');
xlabel('Sample index');
ylabel('Amplitude');
legend('Input: stairs','Filtered: stem');
grid on;
