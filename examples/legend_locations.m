% Fixed legend positions remain attached to each axes when saved.
subplot(2,2,1);plot([1 2 3 4],[1 4 2 3],'b-o');legend('Measured','Location','northwest');title('Top left');
subplot(2,2,2);plot([1 2 3 4],[4 1 3 2],'r--s');legend('Reference','Location','northeast');title('Top right');
subplot(2,2,3);plot([1 2 3 4],[1 4 2 3],'g-.^');legend('Response','Location','southwest');title('Bottom left');
subplot(2,2,4);plot([1 2 3 4],[4 1 3 2],'m:d');legend('Output','Location','southeast');title('Bottom right');
