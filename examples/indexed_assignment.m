% Edit channels, append a filtered channel, then trim the recording.
fs = 1000;
t = (0:999)' / fs;
data = [sin(2*pi*30*t), sin(2*pi*30*t)+0.3*sin(2*pi*220*t)];
original = data;

% The saved original shares storage until data is edited.
data(201:220,2) = 0;
b = fir1(32,0.16,'low');
data(:,3) = filter(b,1,data(:,2));

% Remove the startup interval from every channel and its time vector.
data(1:100,:) = [];
t(1:100) = [];
plot(t,data(:,2),'r:',t,data(:,3),'b-');
title('Edited and filtered recording');
xlabel('Time (seconds)'); ylabel('Amplitude');
legend('Edited input','Filtered');
xlim([0.1 0.3]);
