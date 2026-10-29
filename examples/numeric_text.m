% Strict numeric text exchange. Files are relative to the script/current folder.
t=linspace(0,2,1001)';
signal=sin(2*pi*t);
samples=[t signal];
writematrix(samples,'signal.csv');
loaded=readmatrix('signal.csv');
writematrix(loaded,'signal_semicolon.txt','Delimiter','semi');
semicolons=readmatrix('signal_semicolon.txt','Delimiter',';');
error=norm(loaded(:,2)-semicolons(:,2));
plot(loaded(:,1),loaded(:,2));
title('Numeric text round trip');
xlabel('Time (s)');ylabel('Amplitude');
