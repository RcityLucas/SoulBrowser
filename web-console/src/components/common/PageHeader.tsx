import type { ReactNode } from 'react';
import styles from './PageHeader.module.css';

interface PageHeaderProps {
  title: string;
  description?: string;
  highlight?: string;
  tag?: ReactNode;
  icon?: ReactNode;
  extra?: ReactNode;
}

export default function PageHeader({
  title,
  description,
  highlight,
  tag,
  icon,
  extra,
}: PageHeaderProps) {
  return (
    <div className={styles.header}>
      <div className={styles.main}>
        {(tag || highlight) && (
          <div className={styles.metaRow}>
            {tag && <span className={styles.tag}>{tag}</span>}
            {highlight && <span className={styles.highlight}>{highlight}</span>}
          </div>
        )}
        <div className={styles.titleRow}>
          {icon && <span className={styles.icon}>{icon}</span>}
          <div>
            <h1>{title}</h1>
            {description && <p>{description}</p>}
          </div>
        </div>
      </div>
      {extra && <div className={styles.actions}>{extra}</div>}
    </div>
  );
}
