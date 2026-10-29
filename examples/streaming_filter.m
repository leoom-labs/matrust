% Carry filter state across two chunks of two-channel data.
fs = 1000;
t = (0:999)' / fs;
x = [sin(2*pi*30*t) + 0.3*sin(2*pi*220*t), cos(2*pi*45*t)];
b = fir1(48,100/(fs/2));
[samples,channels] = size(x);

[first,state] = filter(b,1,x(1:400,:));
[second,state] = filter(b,1,x(401:end,:),state);
streamed = [first;second];
whole = filter(b,1,x);
chunk_error = max(abs(whole-streamed))

plot(t,x(:,1),'b:',t,streamed(:,1),'r-');
title('Continuous filtering across chunks');
xlabel('Time (seconds)'); ylabel('Amplitude');
legend('Input','Filtered'); xlim([0.3 0.5]);
% Chunk boundary: 0.4 seconds. State prevents a restart transient.
