% Log-frequency response: original samples remain in the saved figure and CSV.
f = 10.^linspace(1,5,800);
fc = 1000;
gain = -10*log(1+(f/fc).^2)/log(10);
semilogx(f,gain,'b-');
hold on;
semilogx([10 1000 100000],[0 -3.0103 -40.0004],'ro');
title('Low-pass response over four decades');
xlabel('Frequency (Hz)'); ylabel('Magnitude (dB)');
legend('Response','Reference points');
axis([10 100000 -45 3]); grid on;
