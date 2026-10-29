% Relative files follow Current Folder (CLI: this script's folder).
f = figure;
t = 0:0.01:2;
plot(t,sin(2*pi*t));
title('Editable saved signal');
xlabel('Time (s)');
ylabel('Amplitude');
savefig(f,'signal.fig');
savefig(f,'signal.mfig'); % MatRust native format extension
close(f);
reopened = openfig('signal.mfig');
