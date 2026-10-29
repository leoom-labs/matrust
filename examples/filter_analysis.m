% FIR and IIR analysis. Explicit sample counts keep computation bounded.
fs = 1000;
b = fir1(64,0.2);
delay = grpdelay(b,1,1024);
phase = phasez(b,1,1024);
impulse = impz(b,1,256);
step_response = stepz(b,1,256);
% A bare analysis call plots; assignment returns a column vector.
grpdelay(b,1,1024);
% SOS analysis names are MatRust extensions.
sos = butter_sos(8,0.2);
response = sosstepz(sos,256);
t = (0:255)/fs;
plot(t,response);
title('Butterworth step response');
xlabel('Time (s)'); ylabel('Amplitude');
