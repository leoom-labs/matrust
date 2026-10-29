% FIR design and digital filtering. Frequencies are normalized to Nyquist.
fs = 1000;
t = (0:1999)/fs;
x = sin(2*pi*10*t) + 0.3*sin(2*pi*200*t);
b = fir1(64, 0.1, 'low');
y = filter(b, 1, x);
plot(t, y);
title('FIR lowpass: 10 Hz signal with 200 Hz interference removed');
xlabel('Time (seconds)');
ylabel('Amplitude');
% Run freqz(b,1,1024) to inspect the magnitude response.
% SOS alternative: sos=butter_sos(6,0.1); y=sosfilt(sos,x);
