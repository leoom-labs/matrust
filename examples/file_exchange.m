% Run in the editor or with matrust examples/file_exchange.m.
% Saves file_exchange.mat in the desktop Current Folder (CLI: script folder).
t = 0:0.001:1;
signal = sin(2*pi*5*t);
mask = signal > 0;
save file_exchange.mat t signal mask -v7.3;
clear;
load('file_exchange.mat','t','signal','mask');
plot(t,signal);
title('Signal reopened from a MAT file');
xlabel('Time (s)');
ylabel('Amplitude');
